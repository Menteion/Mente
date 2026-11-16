# Runs a step's script as the runner's own shell runs it, bash --noprofile --norc -eo pipefail: the
# workflow policy's run of the secret steps shows here what an earlier step's BASH_ENV, exported
# functions or SHELLOPTS do in that shell, which this script's own bash reads first, as the
# runner's does (the re-check of c55cf8f; the final pass on 89c7691). The bash is this one's own,
# by its path, never one first on PATH.
exec "$BASH" --noprofile --norc -eo pipefail "$1"
