#!/bin/sh
# opencal sync with rclone (Google Drive, Dropbox, S3, WebDAV, SFTP, ... used as plain file storage).
# Set up a remote first with `rclone config`, then in opencal's Settings → Sync use this script as
# both the pull and the push command:
#   /usr/share/doc/opencal/examples/sync/rclone.sh
# $CAL_SYNC_STAGE says which stage is running.
set -u
DIR=${OPENCAL_RCLONE_DIR:-remote:opencal}
FILE=calendar.ics

case "$CAL_SYNC_STAGE" in
pull)
    # Only "not found" means "no remote copy yet" (first sync). Any other error (no network,
    # expired login) must fail, or opencal would push over the remote copy.
    listing=$(rclone lsf "$DIR/")
    rc=$?
    [ "$rc" -eq 3 ] && exit 0          # directory not found
    [ "$rc" -eq 0 ] || exit "$rc"
    printf '%s\n' "$listing" | grep -qx "$FILE" || exit 0
    rclone copyto "$DIR/$FILE" "$CAL_DB_PULL"
    ;;
push)
    rclone copyto "$CAL_DB" "$DIR/$FILE"
    ;;
esac
