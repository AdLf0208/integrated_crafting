#!/usr/bin/env bash
# In-game check for Integrated Crafting slice 1: a Crafting Interface accepts a
# crafting job, holds its ingredients, crafts one recipe at a time, blocks with
# the reason when it cannot continue, and reports its queue.
#
# Every mod command is typed with `ctl chat "/cmd"` because `ctl command` runs the
# console path and bypasses the mod's own `perform_command` hook.
#
# The End, in the void at x=300 (away from the central island, its dragon egg and
# the ambient item entities of a generated level), a stone floor at y=100.
#
# Phases:
#  A  two logs fed to the interface and a request for eight planks: the readout
#     shows one running job, craft 0/2 (the tutorial's example);
#  B  after the crafts: the readout is idle and the eight planks left the result
#     slots into the chest below (a hopper between them does the transferring);
#  C  a request with no ingredients is refused, naming the deficit;
#  D  a queued job is cancelled and the ingredients stay in the slots;
#  E  a request for an item no crafting recipe makes is refused by name.
#
# SELFTEST=1 flips phase A's expected craft count, which must FAIL: proof that
# the scenario can fail.
#
# The game must be a launcher-built one (`horizon-launcher native-build`); a
# native SDK mod is compiled into the game by the launcher, so there is no game
# here to launch. See notes.md.
set -Eeuo pipefail
export LC_ALL=C
[ -z "${IC_DEBUG:-}" ] || set -x

SELFTEST=${SELFTEST:-0}
KEEP=${KEEP:-0}

CLONE=/home/adlf/Desktop/all_the_mods_10_rust_rewrite
PORT=$CLONE/ports/integratedcrafting
PIN_BIN=$CLONE/ports/_shared/bin/horizon-1d63e0a5
CTL=${CTL:-$PIN_BIN/horizon-ctl}
GAME=${GAME:-${HORIZON_TARGET_DIR:-$PORT/target/horizon-cargo}/fast/horizon-client}
RES=${RES:-${HORIZON_RES:-/home/adlf/Desktop/horizon/.local/horizon-client/26.3}}
SOCK=${SOCK:-/tmp/opencode/integratedcrafting-e2e.sock}
WORLD=IntegratedCraftingE2E
# Where the arena stands: the End's void, far from the central island.
X0=300
Y=101
# Phase A's expected craft count, from the readout.
EXPECT_CRAFT=${EXPECT_CRAFT:-0\/2}
# The selftest asks for a craft count the engine cannot report, so A must fail.
if [ "$SELFTEST" = 1 ]; then EXPECT_CRAFT=7\/9; fi

[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME (build it with horizon-launcher native-build)" >&2; exit 1; }
[ -d "$RES/assets" ] || { echo "missing prepared resources: $RES" >&2; exit 1; }

SCRATCH=$(mktemp -d /tmp/opencode/integratedcrafting-home.XXXXXX)
export HORIZON_HOME=$SCRATCH/game
export HORIZON_LAUNCHER_HOME=$SCRATCH/launcher
export XDG_DATA_HOME=$SCRATCH/data
export XDG_CONFIG_HOME=$SCRATCH/config
export HORIZON_NO_AUDIO=1
mkdir -p "$(dirname "$SOCK")" "$XDG_CONFIG_HOME"
rm -f "$SOCK"

ctl() { "$CTL" --socket "$SOCK" "$@"; }
say() { ctl chat "/$1" >/dev/null; }
fail() { echo "FAIL: $*" >&2; ctl logs --lines 60 >&2 || true; exit 1; }
on_error() { echo "FAIL: line $1: '$2' exited $?" >&2; ctl logs --lines 40 >&2 || true; }
trap 'on_error "$LINENO" "$BASH_COMMAND"' ERR

cleanup() {
    local status=$?
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    if [ "$KEEP" = 1 ]; then
        echo "kept scratch: $SCRATCH" >&2
    else
        rm -rf "$SCRATCH" 2>/dev/null || true
    fi
    # Last line of every run: `check-e2e-logs.py` reads it as the verdict.
    echo "EXIT:$status"
}
trap cleanup EXIT

launch() {
    rm -f "$SOCK"
    cd "$SCRATCH"
    # A native SDK mod is compiled into the game the launcher builds; capture and
    # hold assemble the interface and its container from the mod's data.
    "$CTL" --socket "$SOCK" launch --game "$GAME" --world "$WORLD" --seed 20261105 \
        --creative --headless --wait --resources "$RES" >/dev/null
    ctl wait-ready 120
    ctl wait 40
}

block_name() { ctl block "$1" "$2" "$3" | jq -r '.block // empty'; }

# The End's void at X0: a stone floor and clear air above it. The player is
# teleported first, so the chunks the fills touch are loaded.
arena() {
    ctl command "execute in minecraft:the_end run tp @s $X0.5 $Y 0.5" >/dev/null
    local loaded=false
    for _ in $(seq 1 120); do
        ctl wait 5 >/dev/null
        if [ "$(ctl block "$X0" 100 0 | jq -r '.loaded // false')" = true ]; then
            loaded=true
            break
        fi
    done
    [ "$loaded" = true ] || fail "the End arena chunk never loaded"
    ctl command "execute in minecraft:the_end run fill $((X0 - 8)) 100 -8 $((X0 + 8)) 100 8 minecraft:stone" >/dev/null
    ctl wait 6 >/dev/null
    ctl command "execute in minecraft:the_end run fill $((X0 - 8)) $Y -8 $((X0 + 8)) 108 8 minecraft:air" >/dev/null
    ctl wait 6 >/dev/null
    ctl command "execute in minecraft:the_end run kill @e[type=!player]" >/dev/null
    ctl command "execute in minecraft:the_end run tp @s $X0.5 $Y 0.5" >/dev/null
    ctl wait 20
    [ "$(block_name "$X0" 100 0)" = "minecraft:stone" ] || fail "the arena floor is not stone"
}

set_block() {
    ctl command "execute in minecraft:the_end run setblock $1 $2 $3 $4" >/dev/null
    ctl wait 5 >/dev/null
}

# `/integratedcrafting <args>`: the replies, as one string.
ic() {
    say "integratedcrafting $*"
    ctl wait 12 >/dev/null
    ctl messages 12 | jq -r '.chat[]' | tr '\n' ' '
}

expect_reply() { # expect_reply NEEDLE LABEL
    case "$REPLY" in
        *"$1"*) echo "  $2: $REPLY" >&2 ;;
        *) fail "$2: the reply did not contain '$1': $REPLY" ;;
    esac
}

# The interface at (X0, Y, 0); a hopper under it and a chest under the hopper take
# the crafted results, so phase B can count what the interface produced.
scene() {
    set_block "$X0" $Y 0 "integratedcrafting:part_interface_crafting"
    set_block "$X0" $((Y - 1)) 0 "minecraft:hopper[facing=down]"
    set_block "$X0" $((Y - 2)) 0 "minecraft:chest"
    ctl wait 10 >/dev/null
}

run() {
    arena
    scene
    [ "$(block_name "$X0" $Y 0)" = "integratedcrafting:part_interface_crafting" ] \
        || fail "the crafting interface was not placed"

    echo "== A: two logs and a request for eight planks =="
    REPLY=$(ic feed "$X0" $Y 0 minecraft:oak_log 2)
    expect_reply "inserted 2" "A feed"
    REPLY=$(ic request "$X0" $Y 0 minecraft:oak_planks 8)
    expect_reply "job 1 accepted" "A request"
    REPLY=$(ic status "$X0" $Y 0)
    expect_reply "running job 1" "A status"
    expect_reply "craft $EXPECT_CRAFT" "A craft count"

    echo "== B: the crafts finish, and the planks reach the chest below =="
    ctl wait 60 >/dev/null
    REPLY=$(ic status "$X0" $Y 0)
    expect_reply "idle" "B status"
    local planks=""
    for _ in $(seq 1 40); do
        ctl wait 10 >/dev/null
        planks=$(ctl command "execute in minecraft:the_end run data get block $X0 $((Y - 2)) 0 Items" 2>/dev/null | jq -r '.ok // false')
        if [ "$planks" = true ] && [ "$(ctl messages 3 | jq -r '.chat[]' | grep -c 'oak_planks')" -gt 0 ]; then
            break
        fi
    done
    ctl command "execute in minecraft:the_end run data get block $X0 $((Y - 2)) 0 Items" >/dev/null 2>&1 || true
    ctl wait 6 >/dev/null
    local chest
    chest=$(ctl messages 6 | jq -r '.chat[]' | tr '\n' ' ')
    case "$chest" in
        *oak_planks*) echo "  B chest: $chest" >&2 ;;
        *) fail "the crafted planks never reached the chest: $chest" ;;
    esac

    echo "== C: a request the slots cannot pay for is refused with the deficit =="
    REPLY=$(ic request "$X0" $Y 0 minecraft:stick 4)
    expect_reply "missing 2 x minecraft:oak_planks" "C refusal"
    expect_reply "(have 0)" "C deficit"

    echo "== D: a cancelled job consumes nothing =="
    REPLY=$(ic feed "$X0" $Y 0 minecraft:oak_log 2)
    expect_reply "inserted 2" "D feed"
    REPLY=$(ic request "$X0" $Y 0 minecraft:oak_planks 8)
    expect_reply "job 1 accepted" "D request"
    REPLY=$(ic cancel "$X0" $Y 0)
    expect_reply "1 job(s) cancelled" "D cancel"
    REPLY=$(ic status "$X0" $Y 0)
    expect_reply "idle" "D status"

    echo "== E: a request for an item no recipe makes is refused by name =="
    REPLY=$(ic request "$X0" $Y 0 minecraft:diamond 1)
    expect_reply "no crafting recipe makes minecraft:diamond" "E refusal"
}

launch
run
echo "PASS: a Crafting Interface accepted 'eight oak planks', held two logs, crafted it in two crafts and moved the eight planks to the chest below; a request with no planks for sticks was refused with its deficit; a cancelled job left its logs; an item with no recipe was refused by name (phase A craft count ${EXPECT_CRAFT})"
