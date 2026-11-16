#!/bin/sh
# Run another sync script, then show a desktop notification with the number of changes.
#   Push: /usr/share/doc/opencal/examples/sync/notify.sh /usr/share/doc/opencal/examples/sync/rclone.sh
set -eu
"$@"
if [ "$CAL_SYNC_STAGE" = push ]; then
    notify-send "opencal" "Synced: $CAL_SYNC_INCOMING in, $CAL_SYNC_OUTGOING out"
fi
