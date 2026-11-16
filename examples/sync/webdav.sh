#!/bin/sh
# opencal sync with a WebDAV folder (Nextcloud, ownCloud, a NAS, ...).
# Credentials come from ~/.netrc:  machine cloud.example.com login me password app-password
# Use this script as both the pull and the push command.
set -eu
URL=${OPENCAL_WEBDAV_URL:-https://cloud.example.com/remote.php/dav/files/me/opencal/calendar.ics}

case "$CAL_SYNC_STAGE" in
pull)
    code=$(curl -s -n -o "$CAL_DB_PULL" -w '%{http_code}' "$URL")
    case "$code" in
    200) ;;
    404) rm -f "$CAL_DB_PULL" ;; # first sync: no remote copy yet
    *)
        echo "download failed: HTTP $code" >&2
        rm -f "$CAL_DB_PULL"
        exit 1
        ;;
    esac
    ;;
push)
    curl -sf -n -T "$CAL_DB" "$URL"
    ;;
esac
