#!/bin/sh
# Prints the next release version, YYYY.MDD.N, from the tag names on stdin.
# N counts the releases of the given UTC day (default today); other tags are ignored.
# Usage: git tag | scripts/next_version.sh [YYYY-MM-DD]
# Self-test: scripts/next_version.sh --test
set -e

if [ "$1" = --test ]; then
	check() {
		got=$(printf '%s\n' $3 | "$0" "$1")
		[ "$got" = "$2" ] || { echo "FAIL: $1 with tags [$3] gave $got, want $2"; exit 1; }
	}
	check 2026-11-02 2026.1102.1 ""
	check 2026-11-02 2026.1102.2 "2026.1102.1"
	check 2026-11-02 2026.1102.11 "2026.1102.9 2026.1102.10"
	check 2026-11-02 2026.1102.1 "2026.1101.3 2025.1102.4 2026.110.2"
	check 2026-11-02 2026.1102.1 "v0.36.0 v0.35.1 0.36.0 latest"
	check 2027-01-05 2027.105.1 "2026.1231.2 2027.1050.1"
	echo ok
	exit
fi

prefix=$(date -u -d "${1:-now}" +%Y.%-m%d)
awk -F. -v p="$prefix" '$1 "." $2 == p && NF == 3 && $3 ~ /^[0-9]+$/ && $3 > n { n = $3 } END { print p "." n + 1 }'
