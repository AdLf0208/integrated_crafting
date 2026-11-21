#!/usr/bin/env bash
# In-game check for Integrated Crafting: a Crafting Interface accepts a crafting
# job, holds its ingredients, crafts one recipe at a time, blocks with the reason
# when it cannot continue, and reports its queue (slice 1).
#
# Slice 2 adds phase G: a request whose ingredient is itself craftable runs the
# dependency first (a log becomes planks, the planks become sticks) — upstream's
# `CraftingJobDependencyGraph`.
#
# Every mod command is typed with `ctl chat "/cmd"` because `ctl command` runs the
# console path and bypasses the mod's own `perform_command` hook. Every assertion
# reads the chat events that arrived *after* the command was sent (`ctl events
# --since`), never the last N lines: an older reply must never pass for this one.
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
#  E  a request for an item no crafting recipe makes is refused by name;
#  F  feeding items the player does not hold creates nothing;
#  G  one log + a request for four sticks: the acceptance names the two-step plan,
#     the interface crafts planks then sticks, the log is consumed, two planks are
#     left over and four sticks reach the chest below;
#  H  the interface's loot table rolls one interface, and an unknown table rolls
#     nothing (the differential);
#  I  the package's one carried recipe, crafted through the interface from its six
#     ingredients. It names the parent port's items, so this phase runs only in a
#     game with both packages (PARENT_ITEMS=1); a single-package leg skips it.
#
# SELFTEST=1 flips phase A's expected craft count; SELFTEST=dependency flips phase
# G's expected plan. Both must FAIL: proof that the scenario can fail.
#
# The game must be a launcher-built one (`ports/tools/launcher-game.sh build`); a
# native SDK mod is compiled into the game by the launcher, so there is no game
# here to launch. See notes.md.
set -Eeuo pipefail
export LC_ALL=C
[ -z "${IC_DEBUG:-}" ] || set -x

SELFTEST=${SELFTEST:-0}
KEEP=${KEEP:-0}
PARENT_LINE=''
if [ "${PARENT_ITEMS:-0}" = 1 ]; then PARENT_LINE='crafted through the interface'; fi

CLONE=/home/adlf/Desktop/all_the_mods_10_rust_rewrite
PORT=$CLONE/ports/integratedcrafting
CTL=${CTL:-$(dirname "${GAME:-/nonexistent}")/horizon-ctl}
GAME=${GAME:-${HORIZON_TARGET_DIR:-$PORT/target/horizon-cargo}/fast/horizon-client}
RES=${RES:-${HORIZON_RES:-/home/adlf/Desktop/horizon/.local/horizon-client/26.3}}
SOCK=${SOCK:-/tmp/opencode/integratedcrafting-e2e.sock}
WORLD=IntegratedCraftingE2E
# Where the arena stands: the End's void, far from the central island.
X0=300
Y=101
# The second arena spot, for the dependency chain (phase G).
X1=304
# The third, for the interface's own recipe (phase I): its nine cells need nine free slots.
X2=302
# Phase A's expected craft count, from the readout.
EXPECT_CRAFT=${EXPECT_CRAFT:-0\/2}
# Phase G's expected plan, from the acceptance reply.
EXPECT_PLAN=${EXPECT_PLAN:-}
# The selftests ask for values the fixed engine cannot report, so the phase must fail.
if [ "$SELFTEST" = 1 ]; then EXPECT_CRAFT=7\/9; fi
if [ "$SELFTEST" = dependency ]; then EXPECT_PLAN="(crafts: 9x minecraft:oak_planks"; fi

[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME (build it with launcher-game.sh)" >&2; exit 1; }
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

# The chat sequence marker. `events` carries a sequence number (`messages` does
# not), so a reply can be read from the point the command was sent. `ic` runs in
# a command substitution, whose variables do not reach this shell: the sequence
# lives in a file.
SEQ_FILE=$SCRATCH/chat.seq
: > "$SEQ_FILE"
chat_seq() { cat "$SEQ_FILE" 2>/dev/null || echo 0; }
mark() { ctl events | jq -r '.last // 0'; }
set_mark() { printf '%s' "$1" > "$SEQ_FILE"; }
new_chat() { # new_chat SEQ
    ctl events --since "$1" | jq -r '[.events[]? | select(.event == "chat") | .text] | join(" / ")'
}

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

# `/integratedcrafting <args>`: the chat lines that arrived after the command was
# sent, as one string.
ic() {
    local before last got
    before=$(chat_seq)
    say "integratedcrafting $*"
    ctl wait 12 >/dev/null
    got=$(ctl events --since "$before")
    last=$(jq -r '.last // 0' <<<"$got")
    set_mark "$last"
    jq -r '[.events[]? | select(.event == "chat") | .text] | join(" / ")' <<<"$got"
}

expect_reply() { # expect_reply NEEDLE LABEL
    case "$REPLY" in
        *"$1"*) echo "  $2: $REPLY" >&2 ;;
        *) fail "$2: the reply did not contain '$1': $REPLY" ;;
    esac
}

# The chat the console command at `$1` printed, read from a fresh marker.
console_chat() { # console_chat COMMAND
    local before last got
    before=$(chat_seq)
    ctl command "execute in minecraft:the_end run $1" >/dev/null
    ctl wait 4 >/dev/null
    got=$(ctl events --since "$before")
    last=$(jq -r '.last // 0' <<<"$got")
    set_mark "$last"
    jq -r '[.events[]? | select(.event == "chat") | .text] | join(" / ")' <<<"$got"
}

# The chest at `$1 $2 $3` holds exactly `$5` of `$4` once the hopper below the interface has moved
# the result out: poll the fresh `data get` reply (never an old line) and fail if it ever holds
# more than the job made — a hopper under the interface must not refill a stack the interface has
# already handed over. The count is the parity rule's assertion here.
expect_in_chest() { # expect_in_chest X Y Z ITEM COUNT
    local x=$1 y=$2 z=$3 item=$4 want=$5 text="" got=""
    for _ in $(seq 1 60); do
        ctl wait 10 >/dev/null
        text=$(console_chat "data get block $x $y $z Items")
        case "$text" in
            *"$item"*) ;;
            *) continue ;;
        esac
        got=$(sed -n 's/.*count: \([0-9][0-9]*\).*/\1/p' <<<"$text" | head -1)
        [ -n "$got" ] || continue
        [ "$got" -le "$want" ] \
            || fail "the chest at $x,$y,$z holds $got $item, more than the $want the job made: $text"
        if [ "$got" -eq "$want" ]; then
            echo "  chest $x,$y,$z: $got x $item" >&2
            return 0
        fi
    done
    fail "the chest at $x,$y,$z never held $want $item: $text"
}

# The interface at (X0, Y, 0); a hopper under it and a chest under the hopper take
# the crafted results, so phase B can count what the interface produced.
scene() {
    set_block "$X0" $Y 0 "integratedcrafting:part_interface_crafting"
    set_block "$X0" $((Y - 1)) 0 "minecraft:hopper[facing=down]"
    set_block "$X0" $((Y - 2)) 0 "minecraft:chest"
    ctl wait 10 >/dev/null
}

# A second, empty interface at (X1, Y, 0), its own hopper and chest: phase G's
# chain must not see the items the earlier phases left at X0.
scene_two() {
    set_block "$X1" $Y 0 "integratedcrafting:part_interface_crafting"
    set_block "$X1" $((Y - 1)) 0 "minecraft:hopper[facing=down]"
    set_block "$X1" $((Y - 2)) 0 "minecraft:chest"
    ctl wait 10 >/dev/null
}

# The third interface at (X2, Y, 0) for phase I: its nine ingredient slots must all be empty.
scene_three() {
    set_block "$X2" $Y 0 "integratedcrafting:part_interface_crafting"
    set_block "$X2" $((Y - 1)) 0 "minecraft:hopper[facing=down]"
    set_block "$X2" $((Y - 2)) 0 "minecraft:chest"
    ctl wait 10 >/dev/null
}

run() {
    arena
    scene
    scene_two
    [ "$(block_name "$X0" $Y 0)" = "integratedcrafting:part_interface_crafting" ] \
        || fail "the crafting interface was not placed"
    [ "$(block_name "$X1" $Y 0)" = "integratedcrafting:part_interface_crafting" ] \
        || fail "the second crafting interface was not placed"
    # Everything from here on is read from a fresh chat marker: the setup lines
    # (teleports, fills, gives) must never pass for a command's answer.
    set_mark "$(mark)"

    echo "== A: two logs and a request for eight planks =="
    # `/feed` moves the player's own items into the input slots: it creates nothing, so the logs
    # have to be in the inventory first (and are gone from it afterwards).
    ctl give minecraft:oak_log 2 >/dev/null
    ctl wait 6 >/dev/null
    REPLY=$(ic feed "$X0" $Y 0 minecraft:oak_log 2)
    expect_reply "inserted 2" "A feed"
    [ "$(ctl inventory | jq -r '.counts["minecraft:oak_log"] // 0')" = 0 ] \
        || fail "the feed left the player's logs behind: the slots took copies, not the items"
    REPLY=$(ic request "$X0" $Y 0 minecraft:oak_planks 8)
    expect_reply "job 1 accepted" "A request"
    REPLY=$(ic status "$X0" $Y 0)
    expect_reply "running job 1" "A status"
    expect_reply "craft $EXPECT_CRAFT" "A craft count"

    echo "== B: the crafts finish, and the planks reach the chest below =="
    ctl wait 60 >/dev/null
    REPLY=$(ic status "$X0" $Y 0)
    expect_reply "idle" "B status"
    expect_in_chest "$X0" $((Y - 2)) 0 "minecraft:oak_planks" 8

    echo "== C: a request the slots cannot pay for is refused with the deficit =="
    REPLY=$(ic request "$X0" $Y 0 minecraft:stick 4)
    # 26.3's stick recipe takes the whole planks tag, so the deficit names every plank it would
    # accept: the count, the plank this pack uses and the zero it has must all be there.
    expect_reply "missing 2 x" "C refusal"
    expect_reply "minecraft:oak_planks" "C deficit names the plank it would take"
    expect_reply "(have 0)" "C deficit"

    echo "== D: a cancelled job consumes nothing =="
    ctl give minecraft:oak_log 2 >/dev/null
    ctl wait 6 >/dev/null
    REPLY=$(ic feed "$X0" $Y 0 minecraft:oak_log 2)
    expect_reply "inserted 2" "D feed"
    # A's job was 1 and finished; the next accepted job is 2 (read from fresh chat, so this can
    # not match A's line — the previous version of this script did, and its assertion was vacuous).
    REPLY=$(ic request "$X0" $Y 0 minecraft:oak_planks 8)
    expect_reply "job 2 accepted" "D request"
    REPLY=$(ic cancel "$X0" $Y 0)
    expect_reply "1 job(s) cancelled" "D cancel"
    REPLY=$(ic status "$X0" $Y 0)
    expect_reply "idle" "D status"

    echo "== E: a request for an item no recipe makes is refused by name =="
    # `minecraft:bedrock` has no crafting recipe at 26.3 (a diamond does: a shapeless one from a
    # diamond block), so this is the item the refusal is proven on.
    REPLY=$(ic request "$X0" $Y 0 minecraft:bedrock 1)
    expect_reply "no crafting recipe makes minecraft:bedrock" "E refusal"

    echo "== F: feeding items the player does not hold creates nothing =="
    [ "$(ctl inventory | jq -r '.counts["minecraft:oak_log"] // 0')" = 0 ] \
        || fail "the player still holds logs before the refusal phase"
    REPLY=$(ic feed "$X0" $Y 0 minecraft:oak_log 2)
    expect_reply "the player holds no minecraft:oak_log to feed" "F refusal"
    REPLY=$(ic status "$X0" $Y 0)
    expect_reply "idle" "F status"

    echo "== G: one log and a request for four sticks runs the planks first =="
    # Upstream's job dependency graph: the stick recipe needs planks, the interface has none,
    # but the planks are craftable from the log it was fed. The acceptance reply names the whole
    # plan, so the assertion does not depend on the tick the status happens to be read at.
    ctl give minecraft:oak_log 1 >/dev/null
    ctl wait 6 >/dev/null
    REPLY=$(ic feed "$X1" $Y 0 minecraft:oak_log 1)
    expect_reply "inserted 1" "G feed"
    [ "$(ctl inventory | jq -r '.counts["minecraft:oak_log"] // 0')" = 0 ] \
        || fail "the feed copied the log instead of moving it"
    REPLY=$(ic request "$X1" $Y 0 minecraft:stick 4)
    expect_reply "job 1 accepted for 4 x minecraft:stick" "G request"
    expect_reply "minecraft:oak_planks" "G plan names the plank step"
    if [ -n "$EXPECT_PLAN" ]; then
        # Only the selftest sets this: it asks for a plan the engine cannot produce.
        expect_reply "$EXPECT_PLAN" "G plan (selftest)"
    fi
    REPLY=$(ic status "$X1" $Y 0)
    expect_reply "step 1/2" "G chain is running its first step"
    # Planks (20 ticks) then sticks (20 ticks), read as idle rather than on a tick boundary.
    for _ in $(seq 1 20); do
        ctl wait 10 >/dev/null
        REPLY=$(ic status "$X1" $Y 0)
        case "$REPLY" in *idle*) break ;; esac
    done
    case "$REPLY" in
        *idle*) echo "  G status: $REPLY" >&2 ;;
        *) fail "the dependency job never finished: $REPLY" ;;
    esac
    expect_in_chest "$X1" $((Y - 2)) 0 "minecraft:stick" 4
    # The log is consumed and the four planks the step made are spent two at a time by the stick
    # recipe: two planks are left in the ingredient slots, and no log is there any more.
    slots=$(console_chat "data get block $X1 $Y 0 ic_slots")
    case "$slots" in
        *minecraft:oak_planks*) echo "  G leftover planks: $slots" >&2 ;;
        *) fail "the planks step left no planks in the slots: $slots" ;;
    esac
    case "$slots" in
        *minecraft:oak_log*) fail "the log was not consumed by the planks step: $slots" ;;
        *) echo "  G log consumed" >&2 ;;
    esac

    echo "== H: the interface's loot table rolls its own item =="
    # The carried table is the block's self-drop. `/loot give` rolls it directly (creative-breaking
    # the block would test the game's drop path, not the table) and an unknown table is the
    # differential: nothing appears, so the positive cannot be a coincidence of the give path.
    local before after
    before=$(ctl inventory | jq -r '.counts["integratedcrafting:part_interface_crafting"] // 0')
    REPLY=$(console_chat "loot give @s loot integratedcrafting:blocks/part_interface_crafting")
    ctl wait 6 >/dev/null
    after=$(ctl inventory | jq -r '.counts["integratedcrafting:part_interface_crafting"] // 0')
    [ "$after" -eq $((before + 1)) ] \
        || fail "the interface's loot table did not give one interface ($before -> $after): $REPLY"
    echo "  H loot: $before -> $after" >&2
    REPLY=$(console_chat "loot give @s loot integratedcrafting:blocks/no_such_table")
    ctl wait 6 >/dev/null
    after=$(ctl inventory | jq -r '.counts["integratedcrafting:part_interface_crafting"] // 0')
    [ "$after" -eq $((before + 1)) ] \
        || fail "an unknown loot table gave an item ($after): $REPLY"
    echo "  H differential (unknown table): no item" >&2

    echo "== I: the carried recipe, crafted by the interface =="
    # The package's one recipe names the parent port's items. A single-package game does not have
    # them, so the leg is skipped there; with both packages installed (PARENT_ITEMS=1) the recipe
    # must load, be requested, consume its six ingredients and put the interface in the chest.
    # An unknown item id makes `give` fail: that is the single-package case, not an error.
    ctl give integrateddynamics:cable 4 >/dev/null 2>&1 || true
    ctl wait 6 >/dev/null
    local cables
    cables=$(ctl inventory | jq -r '.counts["integrateddynamics:cable"] // 0')
    if [ "$cables" -lt 2 ]; then
        [ "${PARENT_ITEMS:-0}" = 1 ] \
            && fail "PARENT_ITEMS=1 but integrateddynamics:cable is not in this game"
        echo "  I skipped: integrateddynamics:cable is absent from this single-package game" >&2
        return 0
    fi
    # The recipe's nine cells: I C I / V M W / I C I — four iron ingots, two crafting tables, two
    # cables and one menril chunk. A fresh interface has the nine empty slots the grid needs.
    scene_three
    ctl give minecraft:crafting_table 2 >/dev/null
    ctl give minecraft:iron_ingot 4 >/dev/null
    ctl give integrateddynamics:crystalized_menril_chunk 1 >/dev/null
    ctl wait 6 >/dev/null
    REPLY=$(ic feed "$X2" $Y 0 integrateddynamics:cable 2)
    expect_reply "inserted 2" "I feed cable"
    REPLY=$(ic feed "$X2" $Y 0 minecraft:crafting_table 2)
    expect_reply "inserted 2" "I feed crafting table"
    REPLY=$(ic feed "$X2" $Y 0 minecraft:iron_ingot 4)
    expect_reply "inserted 4" "I feed iron"
    REPLY=$(ic feed "$X2" $Y 0 integrateddynamics:crystalized_menril_chunk 1)
    expect_reply "inserted 1" "I feed menril chunk"
    REPLY=$(ic request "$X2" $Y 0 integratedcrafting:part_interface_crafting 1)
    expect_reply "job 1 accepted for 1 x integratedcrafting:part_interface_crafting" "I request"
    for _ in $(seq 1 20); do
        ctl wait 10 >/dev/null
        REPLY=$(ic status "$X2" $Y 0)
        case "$REPLY" in *idle*) break ;; esac
    done
    case "$REPLY" in
        *idle*) echo "  I status: $REPLY" >&2 ;;
        *) fail "the interface's own recipe never finished: $REPLY" ;;
    esac
    expect_in_chest "$X2" $((Y - 2)) 0 "integratedcrafting:part_interface_crafting" 1
    slots=$(console_chat "data get block $X2 $Y 0 ic_slots")
    for consumed in minecraft:crafting_table integrateddynamics:cable \
        integrateddynamics:crystalized_menril_chunk minecraft:iron_ingot; do
        case "$slots" in
            *"$consumed"*) fail "the recipe did not consume $consumed: $slots" ;;
        esac
    done
    echo "  I ingredients consumed: $slots" >&2
}

launch
run
echo "PASS: a Crafting Interface accepted 'eight oak planks' from two logs the player fed it, crafted the planks in two crafts and moved them to the chest below; a feed of items the player does not hold was refused and created nothing; a request with no planks for sticks was refused with its deficit; a cancelled job left its logs; an item with no recipe was refused by name; one log was turned into planks and then four sticks by a job whose plan names both steps, leaving two planks and reaching the chest; the interface's loot table rolled its own item and an unknown table rolled nothing; the carried recipe and the parent's items were ${PARENT_LINE:-not present in this single-package leg} (phase A craft count ${EXPECT_CRAFT})"
