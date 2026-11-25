#!/usr/bin/env bash
# Claude Code PreToolUse hook (matcher: Bash).
# Runs the project's checks before any `git commit` and blocks the commit if they fail.
# Exit 0 allows the command. Exit 2 blocks it, and stderr is shown to Claude.
#
# Which checks run, in the checkout the commit is made in:
#   1. .claude/verify.sh in the repo, if it exists and is executable
#   2. otherwise a guess from the project files (Cargo, Go, npm, pytest)
#   3. otherwise nothing, and the commit is allowed

set -uo pipefail

# Without jq the hook cannot read its input, so it stays out of the way.
command -v jq >/dev/null 2>&1 || exit 0

input=$(cat)
cmd=$(jq -r '.tool_input.command // ""' <<<"$input")
session=$(jq -r '.session_id // "default"' <<<"$input")
cwd=$(jq -r '.cwd // ""' <<<"$input")

# Only gate git commit (also `git -C dir commit` and `... && git commit`).
commit='(^|[;&|(]|[[:space:]])git[[:space:]]+(-[^[:space:]]+[[:space:]]+([^-[:space:]][^[:space:]]*[[:space:]]+)?)*commit([[:space:]]|$)'
grep -Eq "$commit" <<<"$cmd" || exit 0

# Find the checkouts the command commits in. CLAUDE_PROJECT_DIR does not say: in a session of
# a git worktree it is the main checkout. The folder the command starts in says, with the moves
# before the commit and git's own -C. Where those leave more than one folder possible, each is
# checked. Where one is known only once the command runs, the commit is blocked.

unknown() {
  echo "Commit blocked: cannot tell which checkout \`$1\` leads to, so cannot run its checks. Name an existing folder in full: git -C /path/to/checkout commit. If this is only text in the command, such as a commit message, pass it in a file." >&2
  exit 2
}

# The folder that word $3 of the command names, seen from folder $2. Option $1 is -L to resolve
# .. as the shell's cd does, or -P as git's -C does.
folder() {
  local to=$3
  case $to in
    \"*\") to=${to#\"} to=${to%\"} ;;
    \'*\') to=${to#\'} to=${to%\'} ;;
    \~/*) to=$HOME/${to#\~/} ;;
  esac
  case $to in '' | -* | *[\"\'\\\`\$\*\?\[\{\~\<\>]*) return 1 ;; esac
  (unset CDPATH && cd "$1" "$2" && cd "$1" -- "$to" && pwd) 2>/dev/null
}

start=$cwd
[ -d "$start" ] || start=${CLAUDE_PROJECT_DIR:-}
[ -d "$start" ] || exit 0

# A cd that opens the command moves all that follows, unless a lone & sends it to the background.
word="(\"[^\"]*\"|'[^']*'|[^[:space:];&|()\"'])+"
opening="^[[:space:]]*cd[[:space:]]+($word)[[:space:]]*(&&|;|"$'\n'")"
before=${cmd%git*commit*}
amp=${before//&&/} amp=${amp//>&/} amp=${amp//&>/}
while [[ $amp != *'&'* && $before =~ $opening ]]; do
  start=$(folder -L "$start" "${BASH_REMATCH[1]}") || unknown "cd ${BASH_REMATCH[1]}"
  before=${before#"${BASH_REMATCH[0]}"}
done

# A later one may be in a subshell or never run, so the folder before it stays possible. That
# covers popd, which only goes back to one of them.
here=("$start")
while IFS= read -r move; do
  [[ $move =~ (cd|pushd|chdir)[[:space:]]*(.*)$ ]]
  move=${BASH_REMATCH[0]}
  found=
  for from in "${here[@]}"; do
    to=$(folder -L "$from" "${BASH_REMATCH[2]}") || continue
    found=1
    for known in "${here[@]}"; do
      [ "$known" = "$to" ] && continue 2
    done
    here+=("$to")
  done
  [ -n "$found" ] || unknown "$move"
done < <(grep -Eo "(^|[^[:alnum:]_./-])(cd|pushd|chdir)([[:space:]]+$word|[[:space:]]+[\"'(]|[[:space:]]*(\$|[;&|)<>]))" <<<"$before")

setting='(^|[^[:alnum:]_])GIT_(DIR|WORK_TREE|COMMON_DIR)='
setting+='|(^|[^[:alnum:]_.-])env([[:space:]]+(-u[[:space:]]+[^[:space:]]+|-[^[:space:]]+|[[:alnum:]_]+=[^[:space:]]*))*[[:space:]]+(-[[:alnum:]]*C|--chdir)'
if set_by=$(grep -Eo "$setting" <<<"$before"); then
  unknown "$(head -n 1 <<<"$set_by")"
fi

tops=()
while IFS= read -r call; do
  dirs=("${here[@]}")
  read -ra words <<<"$call"
  for ((i = 1; i < ${#words[@]}; i++)); do
    case ${words[i]} in
      -C)
        i=$((i + 1))
        next=()
        for dir in "${dirs[@]}"; do
          to=$(folder -P "$dir" "${words[i]}") && next+=("$to")
        done
        [ ${#next[@]} -gt 0 ] || unknown "git -C ${words[i]}"
        dirs=("${next[@]}")
        ;;
      --git-dir* | --work-tree* | core.worktree=*) unknown "git ${words[i]}" ;;
    esac
  done
  for dir in "${dirs[@]}"; do
    top=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) || continue
    for known in ${tops[@]+"${tops[@]}"}; do
      [ "$known" = "$top" ] && continue 2
    done
    tops+=("$top")
  done
done < <(grep -Eo "$commit" <<<"$cmd"; grep -Eo "${commit%(*}[;&|)]" <<<"$cmd")

# Outside any checkout, the project's own is checked: leaving it must not skip the checks.
if [ ${#tops[@]} -eq 0 ]; then
  top=$(git -C "${CLAUDE_PROJECT_DIR:-$start}" rev-parse --show-toplevel 2>/dev/null) || exit 0
  tops=("$top")
fi

# A change must not edit the check that judges it.
for top in "${tops[@]}"; do
  if [ -n "$(git -C "$top" status --porcelain -- .claude/verify.sh)" ]; then
    echo "Commit blocked: .claude/verify.sh has uncommitted changes. Do not change the checks that judge your change. Ask the user to review and commit verify.sh themselves." >&2
    exit 2
  fi
done

state="${TMPDIR:-/tmp}/claude-commit-gate-${session//[^A-Za-z0-9_-]/}"
checked=

for top in "${tops[@]}"; do
  cd "$top" || continue

  if [ -x .claude/verify.sh ]; then
    check='./.claude/verify.sh'
  elif [ -f Cargo.toml ]; then
    check='cargo check -q && cargo test -q'
  elif [ -f go.mod ]; then
    check='go vet ./... && go test ./...'
  elif [ -f package.json ] && jq -e '(.scripts.test // "") as $t | $t != "" and ($t | test("no test specified") | not)' package.json >/dev/null 2>&1; then
    check='npm test --silent'
  elif [ -f pytest.ini ] || grep -q '^\[tool\.pytest' pyproject.toml 2>/dev/null; then
    check='python3 -m pytest -q'
  else
    continue
  fi
  checked=1

  out=$(bash -c "$check" 2>&1) && continue

  n=$(( $(cat "$state" 2>/dev/null || echo 0) + 1 ))
  echo "$n" >"$state"

  {
    echo "Commit blocked: \`$check\` failed (attempt $n)."
    echo "Ran in $top."
    echo "--- last 60 lines ---"
    tail -n 60 <<<"$out"
    echo "---"
    if [ "$n" -ge 3 ]; then
      echo "This check has failed $n times in a row. Stop. Do not commit. Report to the user what fails, what you tried, and your best guess at the cause."
    else
      echo "Fix the code, not the tests or checks. If the failure is flaky or was already failing before your change, stop and tell the user instead of working around it."
    fi
  } >&2
  exit 2
done

# Every check passed, so the failures are no longer in a row.
[ -z "$checked" ] || rm -f "$state"
exit 0
