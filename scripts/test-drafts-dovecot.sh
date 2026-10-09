#!/usr/bin/env bash
# Disposable real IMAP integration, loopback only. SMTP is an in-process sink.
# No user accounts, keychain items, persistent volumes, or external delivery.
set -euo pipefail
cd "$(dirname "$0")/.."
image='dovecot/dovecot:2.4.2@sha256:3fe299453447826694a42f7bb04b8ea12bc36bbc353fac154df05a0e7d6eaf48'
test_dir="$(mktemp -d)"
container=''
cleanup() {
    if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null 2>&1 || true; fi
    rm -rf "$test_dir"
}
trap cleanup EXIT
for mode in root dot colon; do
    case "$mode" in
        root) prefix=''; separator='/' ;;
        dot) prefix='INBOX.'; separator='.' ;;
        colon) prefix='Personal:'; separator=':' ;;
    esac
    cat > "$test_dir/$mode.conf" <<EOF
auth_allow_cleartext = yes
namespace inbox {
    prefix = "$prefix"
    separator = $separator
    inbox = yes
}
# These test logs must not include message metadata.
mail_plugins {
    mail_log = no
}
EOF
    chmod 644 "$test_dir/$mode.conf"
    container="torromail-drafts-$$-$mode"
    docker run --detach --name "$container" \
        -p 127.0.0.1::31143 \
        -v "$test_dir/$mode.conf:/etc/dovecot/conf.d/zz-torromail-test.conf:ro" \
        "$image" >/dev/null
    port=''
    for attempt in {1..30}; do
        port="$(docker port "$container" 31143/tcp 2>/dev/null | sed 's/.*://')" || true
        [[ -n "$port" ]] && break
        sleep .1
    done
    if [[ -z "$port" ]]; then
        docker logs "$container" 2>&1 | rg 'Error:|Fatal:|Panic:' || true
        echo 'Dovecot test port was not published' >&2
        exit 1
    fi
    python3 - "$port" <<'PY'
import socket, sys, time
for _ in range(60):
    try:
        with socket.create_connection(('127.0.0.1', int(sys.argv[1])), timeout=1) as connection:
            if connection.recv(1024).startswith(b'* OK'):
                break
    except OSError:
        pass
    time.sleep(.25)
else:
    raise SystemExit('Dovecot test server did not start')
PY
    echo "Real Dovecot test: $mode namespace"
    if ! TORROMAIL_TEST_IMAP_PORT="$port" TORROMAIL_TEST_DRAFTS_MAILBOX="${prefix}Drafts" \
        cargo test -p torromail-mcp --test drafts_dovecot -- --ignored --nocapture; then
        docker logs "$container" 2>&1 | rg 'Error:|Fatal:|Panic:' || true
        exit 1
    fi
    docker rm -f "$container" >/dev/null
    container=''
done
