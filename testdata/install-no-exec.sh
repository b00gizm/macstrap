#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
src="$root/scripts/install.sh"
tail=$(awk '/^case ":\$PATH:"/,0' "$src")

bindir=$(mktemp -d)
trap 'rm -rf "$bindir"' EXIT
NAME=macstrap
printf '#!/bin/sh\necho EXEC\n' > "$bindir/$NAME"
chmod +x "$bindir/$NAME"

PATH="$bindir:/usr/bin:/bin"
export PATH bindir NAME

out=$(sh -c "$tail")
case "$out" in
*EXEC*)
	echo "fail: no-args path exec'd the binary" >&2
	exit 1
	;;
esac
case "$out" in
*"installed to $bindir/$NAME"*) ;;
*)
	echo "fail: no-args path did not print install location" >&2
	exit 1
	;;
esac
case "$out" in
*"run macstrap to bootstrap"*) ;;
*)
	echo "fail: no-args path did not tell the user to run macstrap" >&2
	exit 1
	;;
esac

out=$(sh -c "$tail" sh --yes)
case "$out" in
*EXEC*) ;;
*)
	echo "fail: args path did not exec the binary" >&2
	exit 1
	;;
esac

echo ok
