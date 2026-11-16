#!/bin/sh
# opencal sync with a server over ssh. Needs key-based login (no password prompt).
# Use this script as both the pull and the push command.
set -u
HOST=${OPENCAL_SSH_HOST:-user@example.com}
REMOTE_FILE=${OPENCAL_SSH_FILE:-opencal/calendar.ics}

case "$CAL_SYNC_STAGE" in
pull)
    # `test -e` exits 1 for a missing file (first sync) and ssh exits 255 when it can't
    # connect; only the first may count as "no remote copy yet".
    ssh "$HOST" test -e "$REMOTE_FILE"
    rc=$?
    [ "$rc" -eq 1 ] && exit 0
    [ "$rc" -eq 0 ] || exit "$rc"
    scp -q "$HOST:$REMOTE_FILE" "$CAL_DB_PULL"
    ;;
push)
    set -e
    ssh "$HOST" mkdir -p "$(dirname "$REMOTE_FILE")"
    # Upload next to the file, then rename, so a reader never sees half a file.
    scp -q "$CAL_DB" "$HOST:$REMOTE_FILE.tmp"
    ssh "$HOST" mv "$REMOTE_FILE.tmp" "$REMOTE_FILE"
    ;;
esac
