# Sources memory-guard.sh in a subshell, as the guard ran when it lost itself, with `ps` naming a
# process of the owner $1 that is gone, `sleep` counting the rounds instead of waiting, at most $3,
# and `sudo` saying what it was asked, against the stand-in meminfo $2; then says how many rounds
# the guard went on through. The run tests (.github/scripts/policy-run.test.mjs) run this script rather than hand bash a text.
owner="$1"
meminfo="$2"
most="$3"
rounds=0
sleep() { rounds=$((rounds + 1)); [ "$rounds" -le "$most" ]; }
ps() { case "$*" in *uid=*) echo " ${owner}" ;; *) echo "999999 1" ;; esac; }
sudo() { echo "sudo $*"; }
(. .github/scripts/memory-guard.sh "$meminfo"; echo "the guard went on through $((rounds - 1)) rounds")
