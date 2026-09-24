#!/bin/bash
# A stand-in for GitHub's gh CLI. Pull requests are files in $FAKE_GH_DIR named after their branch
# ("/" as "__"), each holding "<number> <base> <state>". Every call is appended to $FAKE_GH_DIR/calls.
dir="$FAKE_GH_DIR"
echo "$*" >> "$dir/calls"
key() { echo "${1//\//__}"; }
case "$1 $2" in
"pr view")
    file="$dir/$(key "$3")"
    [ -f "$file" ] || { echo "no pull requests found for branch \"$3\"" >&2; exit 1; }
    read -r number base state < "$file"
    echo "{\"number\":$number,\"url\":\"https://example.test/pull/$number\",\"baseRefName\":\"$base\",\"state\":\"$state\"}"
    ;;
"pr create")
    shift 2
    while [ $# -gt 0 ]; do
        case "$1" in --head) head="$2"; shift ;; --base) base="$2"; shift ;; esac
        shift
    done
    number=$(( $(find "$dir" -type f ! -name calls | wc -l) + 1 ))
    echo "$number $base OPEN" > "$dir/$(key "$head")"
    echo "https://example.test/pull/$number"
    ;;
"pr edit")
    wanted="$3"; shift 3
    while [ $# -gt 0 ]; do
        case "$1" in --base) base="$2"; shift ;; esac
        shift
    done
    for file in "$dir"/*; do
        [ "$(basename "$file")" = calls ] && continue
        read -r number old state < "$file"
        [ "$number" = "$wanted" ] && echo "$number $base $state" > "$file"
    done
    ;;
*)
    echo "fake gh: unsupported: $*" >&2; exit 2 ;;
esac
