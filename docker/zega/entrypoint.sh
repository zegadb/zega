#!/bin/sh
# Refuse to boot without a bearer token, then serve on 0.0.0.0:9342.
set -eu

token_file=/etc/zega/token
if [ -n "${ZEGA_TOKEN:-}" ]; then
    case "$ZEGA_TOKEN" in
        *[[:space:]]*)
            echo "zega: ZEGA_TOKEN must be a single token" >&2
            exit 1
            ;;
    esac
    umask 077
    printf '%s\n' "$ZEGA_TOKEN" > "$token_file"
    chown zega:zega "$token_file"
elif [ ! -s "$token_file" ]; then
    echo "zega: set ZEGA_TOKEN or mount a token file at /etc/zega/token" >&2
    exit 1
fi

chown zega:zega /var/lib/zega
exec runuser -u zega -- zega-server start \
    --host 0.0.0.0 \
    --port 9342 \
    --data /var/lib/zega \
    --token-file "$token_file"
