#!/bin/sh
# Release build + local perf budgets: cold start, idle memory and frames, a 100k-row result, binary size.
# Needs podman-compose (Postgres from compose.yaml), sqlite3, jq and a display. Exit 1 on any FAIL.
set -e
cd "$(dirname "$0")/.."
perf=target/perf
bin=target/release/queryx

cargo build --release

podman-compose up -d >/dev/null
container=$(podman-compose ps -q 2>/dev/null | head -n 1)
psql() { podman exec -i "$container" psql -v ON_ERROR_STOP=1 -qAt -U queryx queryx "$@"; }
tries=0
until psql -c 'select 1' >/dev/null 2>&1; do
    tries=$((tries + 1))
    [ "$tries" -lt 60 ] || { echo "Postgres did not accept connections"; exit 1; }
    sleep 1
done

psql <<'SQL'
set client_min_messages = warning;
create table if not exists bench_rows (
    id int primary key, name text, created_at timestamptz, amount numeric(12,2), active bool,
    payload jsonb, note text, category text, score double precision, tag text
);
insert into bench_rows
select g, 'name ' || g, timestamptz '2024-01-01 00:00:00+00' + g * interval '1 minute',
       (g % 100000) / 100.0, g % 2 = 0, jsonb_build_object('id', g, 'k', 'v' || g % 10),
       case when g % 2 = 0 then 'note ' || g end, 'category ' || g % 20, g * 0.5, 'tag' || g % 7
from generate_series(1, 100000) g
where not exists (select 1 from bench_rows);
SQL

mkdir -p "$perf"
db="$perf/bench.sqlite"
if [ ! -f "$db" ]; then
    sqlite3 "$db" <<'SQL'
create table bench_rows (
    id integer primary key, name text, created_at text, amount numeric, active integer,
    payload text, note text, category text, score real, tag text
);
with recursive s(g) as (select 1 union all select g + 1 from s where g < 100000)
insert into bench_rows
select g, 'name ' || g, strftime('%Y-%m-%d %H:%M:%S+00', '2024-01-01', '+' || g || ' minutes'),
       round((g % 100000) / 100.0, 2), g % 2 = 0, json_object('id', g, 'k', 'v' || g % 10),
       case when g % 2 = 0 then 'note ' || g end, 'category ' || g % 20, g * 0.5, 'tag' || g % 7
from s;
SQL
fi

export XDG_CONFIG_HOME="$PWD/$perf/config"
rm -rf "$XDG_CONFIG_HOME"
mkdir -p "$XDG_CONFIG_HOME"
export QUERYX_BENCH_PG="${QUERYX_BENCH_PG:-postgres://queryx:queryx@localhost:55432/queryx}"
export QUERYX_BENCH_SQLITE="$PWD/$db"

# Runs scenario $1 and prints its JSON report line; a failed run says so on stderr.
scenario() {
    out=$("$bin" --bench-scenario "$1") || echo "scenario $1 exited $?" >&2
    printf '%s\n' "$out" | tail -n 1
}

spawned=$(date +%s%3N)
startup=$(scenario startup)
idle=$(scenario idle)
big=$(scenario big_result)

failed=0
# row <metric> <value> <budget text> <jq test on .v>
row() {
    if jq -en --argjson v "$2" "$4" >/dev/null; then verdict=PASS; else verdict=FAIL; failed=1; fi
    printf '%-26s %12s  %-14s %s\n' "$1" "$2" "$3" "$verdict"
}
printf '%-26s %12s  %-14s %s\n' metric value budget result
row "cold start ms" "$(echo "$startup" | jq ".first_frame_epoch_ms - $spawned")" "< 400" '$v < 400'
row "startup rss_anon_mb" "$(echo "$startup" | jq .rss_anon_mb)" "< 40" '$v < 40'
row "idle rss_anon_mb" "$(echo "$idle" | jq .rss_anon_mb)" "< 40" '$v < 40'
row "idle_frames" "$(echo "$idle" | jq .idle_frames)" "== 0" '$v == 0'
row "big_result rss_anon_mb" "$(echo "$big" | jq .rss_anon_mb)" "< 120" '$v < 120'
row "big_result rows" "$(echo "$big" | jq .rows)" "== 100000" '$v == 100000'
row "binary bytes" "$(stat -c %s "$bin")" "< 35 MiB" '$v < 35 * 1024 * 1024'
exit "$failed"
