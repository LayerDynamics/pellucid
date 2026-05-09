#!/bin/sh
# pellucid-edge entrypoint — Litestream-managed lifecycle.
#
# 1. If `LITESTREAM_REPLICA_URL` is unset OR empty, skip Litestream
#    entirely and exec the binary directly. Lets local Docker tests
#    and Railway-staging-without-replica deployments work without
#    forcing every operator to provision a replica bucket.
#
# 2. Otherwise, restore the SQLite database from the configured
#    replica IF the local file is missing (cold container boot or
#    fresh Railway Volume). `-if-replica-exists` makes this a no-op
#    when the replica is empty (first deploy of a brand-new
#    bucket), so the binary still starts and the first replicate
#    cycle creates the bucket layout.
#
# 3. Run `litestream replicate -exec`, which spawns the edge binary
#    as a child process and continuously ships its WAL to the
#    replica. When the binary exits, Litestream flushes any
#    outstanding frames and exits with the same code. Signals
#    received from `tini` (SIGTERM on Railway shutdown) propagate
#    through Litestream to the binary, so the SQLite WAL is
#    closed cleanly before the container stops.

set -eu

DB_PATH="/data/pellucid-edge.db"
CONFIG="/etc/litestream/litestream.yml"
BIN="/usr/local/bin/pellucid-edge-bin"

if [ -z "${LITESTREAM_REPLICA_URL:-}" ]; then
    echo "entrypoint: LITESTREAM_REPLICA_URL unset — running edge binary without replication"
    exec "$BIN"
fi

echo "entrypoint: Litestream enabled — replica URL=${LITESTREAM_REPLICA_URL%%\?*}"

# Restore from replica if the local DB doesn't exist yet.
# `-if-replica-exists` exits 0 (silent) when the bucket has no
# snapshot, which is the cold-start case for a brand-new bucket.
if [ ! -f "$DB_PATH" ]; then
    echo "entrypoint: $DB_PATH missing — attempting restore from replica"
    /usr/local/bin/litestream restore \
        -if-replica-exists \
        -config "$CONFIG" \
        "$DB_PATH"
fi

# `replicate -exec` runs the binary as a Litestream-managed child
# process. Stdout/stderr from the binary pass through unchanged.
exec /usr/local/bin/litestream replicate \
    -config "$CONFIG" \
    -exec "$BIN"
