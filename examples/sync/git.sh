#!/bin/sh
# opencal sync through a git repository (GitHub, Gitea, a bare repository over ssh, ...).
# Clone the repository once to ~/calendar-repo (or set OPENCAL_GIT_REPO), then use this script as
# both the pull and the push command. Each push is one commit whose message lists the changes
# ($CAL_SYNC_REPORT).
set -eu
REPO=${OPENCAL_GIT_REPO:-$HOME/calendar-repo}
FILE=calendar.ics

case "$CAL_SYNC_STAGE" in
pull)
    git -C "$REPO" pull --quiet --ff-only
    if [ -e "$REPO/$FILE" ]; then
        cp "$REPO/$FILE" "$CAL_DB_PULL"
    fi
    ;;
push)
    cp "$CAL_DB" "$REPO/$FILE"
    git -C "$REPO" add "$FILE"
    if ! git -C "$REPO" diff --cached --quiet; then
        git -C "$REPO" commit --quiet -F "$CAL_SYNC_REPORT"
    fi
    git -C "$REPO" push --quiet
    ;;
esac
