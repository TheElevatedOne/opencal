#!/bin/sh
# opencal sync with a server over ssh. Needs key-based login (no password prompt).
# Use this script as both the pull and the push command.
set -eu
HOST=${OPENCAL_SSH_HOST:-user@example.com}
REMOTE_FILE=${OPENCAL_SSH_FILE:-opencal/calendar.ics}

case "$CAL_SYNC_STAGE" in
pull)
    if ssh "$HOST" test -e "$REMOTE_FILE"; then
        scp -q "$HOST:$REMOTE_FILE" "$CAL_DB_PULL"
    fi
    ;;
push)
    ssh "$HOST" mkdir -p "$(dirname "$REMOTE_FILE")"
    # Upload next to the file, then rename, so a reader never sees half a file.
    scp -q "$CAL_DB" "$HOST:$REMOTE_FILE.tmp"
    ssh "$HOST" mv "$REMOTE_FILE.tmp" "$REMOTE_FILE"
    ;;
esac
