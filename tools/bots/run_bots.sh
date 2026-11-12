#!/usr/bin/env bash
# Runs the Pagan test bot (test command bot:<seed>,<ticks>) from many
# savegames for a fixed time, several at once; with the mode "explore" the
# exploring bot (explore:<seed>,<ticks>), which plays as a player would.
# usage: tools/bots/run_bots.sh <minutes> <workers> [bot|explore]
# The work folder (PAGAN_BOT_DIR, default target/bots) needs:
#   pagan_bot.exe  a copy of the game (so the build can go on meanwhile)
#   starts/        savegames to start from (save_E_<n>.sav)
# Findings: logs/*.log (lines "BUG ..."), savegames of findings in found/;
# the protocol of each run (what it did, see src/testcmd/botlog.rs) in
# logs/*.jsonl, to look at with tools/bots/protocol.py.
# Then: python tools/bots/collect.py <work folder> <list.md>
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BOT="${PAGAN_BOT_DIR:-$ROOT/target/bots}"
MINUTES=${1:-30}
WORKERS=${2:-6}
MODE=${3:-bot}
case "$MODE" in bot|explore) ;; *) echo "mode must be bot or explore" >&2; exit 1 ;; esac
TICKS=40000
END=$(( $(date +%s) + MINUTES * 60 ))
cd "$ROOT" || exit 1
mkdir -p "$BOT/logs"
STARTS=($(ls "$BOT/starts" | sed 's/save_E_//;s/\.sav//' | sort -n))

worker() {
  local w=$1 job=0
  while [ "$(date +%s)" -lt "$END" ]; do
    local i=$(( (w + job * WORKERS) % ${#STARTS[@]} ))
    local slot=${STARTS[$i]}
    local seed=$(( w * 1000 + job ))
    local dir="$BOT/run/w$w"
    rm -rf "$dir"; mkdir -p "$dir"
    cp "$BOT/starts/save_E_$slot.sav" "$dir/"
    local log="$BOT/logs/${MODE}_slot${slot}_seed${seed}.log"
    echo "start slot $slot seed $seed" > "$log"
    PAGAN_SAVE_DIR="$dir" PAGAN_BOT_LOG="${log%.log}.jsonl" timeout 300 "$BOT/pagan_bot.exe" --no-sound --no-menu \
      --screenshot "$dir/end.png" --do "loadslot:$slot;$MODE:$seed,$TICKS" >> "$log" 2>&1
    echo "exit $?" >> "$log"
    # keep the savegames of findings
    if grep -q "^BUG" "$log"; then
      mkdir -p "$BOT/found/${MODE}_slot${slot}_seed${seed}"
      cp "$dir"/save_E_9[0-9][0-9].sav "$BOT/found/${MODE}_slot${slot}_seed${seed}/" 2>/dev/null
    fi
    job=$((job + 1))
  done
}

for w in $(seq 0 $((WORKERS - 1))); do worker "$w" & done
wait
echo "bots done: $(ls "$BOT/logs"/*.log | wc -l) runs"
