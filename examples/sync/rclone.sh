#!/bin/sh
# opencal sync with rclone (Google Drive, Dropbox, S3, WebDAV, SFTP, ... used as plain file storage).
# Set up a remote first with `rclone config`, then in opencal's Sync Settings use this script as
# both the pull and the push command:
#   /usr/share/doc/opencal/examples/sync/rclone.sh
# $CAL_SYNC_STAGE says which stage is running.
set -eu
REMOTE=${OPENCAL_RCLONE_REMOTE:-remote:opencal/calendar.ics}

case "$CAL_SYNC_STAGE" in
pull)
    # No remote file yet (first sync): succeed without creating $CAL_DB_PULL.
    if [ -z "$(rclone lsf "$REMOTE" 2>/dev/null)" ]; then
        exit 0
    fi
    rclone copyto "$REMOTE" "$CAL_DB_PULL"
    ;;
push)
    rclone copyto "$CAL_DB" "$REMOTE"
    ;;
esac
