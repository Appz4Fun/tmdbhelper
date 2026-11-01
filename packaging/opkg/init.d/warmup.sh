#!/bin/sh
# Shared start and stop logic for the warmup-rs Entware init scripts. Each
# S script sets ENABLED, NAME and MODE, then sources this file.
#
# These scripts are for systems without systemd. On CoreELEC, use the systemd
# units and leave ENABLED=no, so the same crawl doesn't run twice.

ENV_FILE=/opt/etc/warmup-rs.env
PID_FILE=/opt/var/run/$NAME.pid
LOG_FILE=/opt/var/log/$NAME.log
BIN=/opt/bin/warmup-rs

running() {
    [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null
}

start() {
    if [ "$ENABLED" != yes ]; then
        echo "$NAME is disabled. Set ENABLED=yes in $0 to start it."
        return 0
    fi
    if running; then
        echo "$NAME is already running."
        return 0
    fi
    mkdir -p /opt/var/run /opt/var/log
    (
        set -a
        # shellcheck disable=SC1090
        [ -f "$ENV_FILE" ] && . "$ENV_FILE"
        set +a
        RUST_LOG=${RUST_LOG:-warmup_rs=info,warn}
        export RUST_LOG
        exec nice -n 10 "$BIN" --mode="$MODE" >>"$LOG_FILE" 2>&1
    ) &
    echo $! >"$PID_FILE"
    sleep 1
    if running; then
        echo "Started $NAME. The log is in $LOG_FILE."
    else
        rm -f "$PID_FILE"
        echo "$NAME stopped right after it started. See $LOG_FILE."
        return 1
    fi
}

stop() {
    if ! running; then
        rm -f "$PID_FILE"
        echo "$NAME isn't running."
        return 0
    fi
    pid=$(cat "$PID_FILE")
    kill "$pid"
    # Give the crawl 30 seconds to finish its writes, like the systemd units.
    i=0
    while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 30 ]; do
        sleep 1
        i=$((i + 1))
    done
    kill -0 "$pid" 2>/dev/null && kill -9 "$pid"
    rm -f "$PID_FILE"
    echo "Stopped $NAME."
}

case "$1" in
    start) start ;;
    stop | kill) stop ;;
    restart)
        stop
        start
        ;;
    check | status)
        if running; then
            echo "$NAME is running."
        else
            echo "$NAME isn't running."
            exit 1
        fi
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|check}"
        exit 1
        ;;
esac
