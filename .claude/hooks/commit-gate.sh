#!/usr/bin/env bash
# Claude Code hook, registered for three events:
#   PostToolUse (matcher Skill) and UserPromptSubmit: note that this session ran the verify
#     or code-review skill (Claude called it, or the user typed /verify).
#   PreToolUse (matcher Bash): gate `git commit` and `git push`.
# Exit 0 allows. Exit 2 blocks, and stderr is shown to Claude.
#
# git commit is allowed when any of these hold:
#   - this session ran the verify or code-review skill
#   - the commit message says why it was not verified: `Verified: none (<reason>)`
#   - a merge, cherry-pick, revert, or rebase is in progress
#   - commits were already blocked twice in this session, so Claude does not get stuck
# It is always blocked while .claude/verify.sh has uncommitted changes.
#
# git push runs the repo's checks first, only in a repo without CI config (with CI, CI checks
# the PR). The checks are .claude/verify.sh, or a guess from the project files. A failing check
# blocks the push every time. A tree that already passed in this session is not checked again.
#
# LIM_COMMIT_GATE=off|observe|enforce (default enforce). observe writes what would have been
# blocked to .git/lim-commit-gate.log and allows it.

set -uo pipefail

# Without jq the hook cannot read its input, so it stays out of the way.
command -v jq >/dev/null 2>&1 || exit 0

mode=${LIM_COMMIT_GATE:-enforce}
[ "$mode" = off ] && exit 0

input=$(cat)
event=$(jq -r '.hook_event_name // "PreToolUse"' <<<"$input")
session=$(jq -r '.session_id // "default"' <<<"$input")
state="${TMPDIR:-/tmp}/claude-commit-gate-${session//[^A-Za-z0-9_-]/}"

case $event in
  PostToolUse)
    jq -r '.tool_input.skill // ""' <<<"$input" | grep -Eq '(^|:)(verify|code-review)$' && touch "$state.verified"
    exit 0 ;;
  UserPromptSubmit)   # print nothing: stdout here would be added to Claude's context
    jq -r '.prompt // ""' <<<"$input" | grep -Eq '^[[:space:]]*/([^[:space:]]*:)?(verify|code-review)([[:space:]]|$)' && touch "$state.verified"
    exit 0 ;;
  PreToolUse) ;;
  *) exit 0 ;;
esac

cmd=$(jq -r '.tool_input.command // ""' <<<"$input")
cwd=$(jq -r '.cwd // ""' <<<"$input")
git_does() {  # git $1 (also `git -C dir $1` and `... && git $1`)
  grep -Eq "(^|[;&|(]|[[:space:]])git[[:space:]]+(-[^[:space:]]+[[:space:]]+([^-[:space:]][^[:space:]]*[[:space:]]+)?)*$1([[:space:]]|\$)" <<<"$cmd"
}
git_does commit || git_does push || exit 0

cd "${CLAUDE_PROJECT_DIR:-$cwd}" 2>/dev/null || exit 0
top=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$top" || exit 0

block() {  # $1: reason for the log, $2: message for Claude. In observe mode, log and carry on.
  if [ "$mode" = observe ]; then
    printf '%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$session" "$1" "$(head -c 200 <<<"$cmd" | tr '\t\n' '  ')" \
      >>"$(git rev-parse --git-path lim-commit-gate.log)"
    return 0
  fi
  echo "$2" >&2
  exit 2
}

commit_allowed() {
  for p in MERGE_HEAD CHERRY_PICK_HEAD REVERT_HEAD rebase-merge rebase-apply; do
    [ -e "$(git rev-parse --git-path "$p")" ] && return 0
  done
  grep -Eq 'Verified:[[:space:]]*none[[:space:]]*\([^)]*[^)[:space:]][^)]*\)' <<<"$cmd" && return 0
  [ -e "$state.verified" ] && return 0
  [ "$mode" = observe ] && return 1
  local n
  n=$(cat "$state.blocks" 2>/dev/null || echo 0)
  if [ "$n" -ge 2 ]; then rm -f "$state.blocks"; return 0; fi
  echo $((n + 1)) >"$state.blocks"
  return 1
}

if git_does commit; then
  # A change must not edit the check that judges it.
  if [ -e .claude/verify.sh ] && [ -n "$(git status --porcelain -- .claude/verify.sh)" ]; then
    block verify.sh-changed "Commit blocked: .claude/verify.sh has uncommitted changes. Do not change the checks that judge your change. Ask the user to review and commit verify.sh themselves."
  fi
  commit_allowed || block no-verify "Commit blocked: this session has not run the verify skill. Run it before the first commit (AGENTS.md Order of work 2). If this commit does not need verifying, end its message with \`Verified: none (<reason>)\`."
fi
git_does push || exit 0

# git push: with CI, CI checks the PR.
set -- .github/workflows/*.yml .github/workflows/*.yaml
for f in "$@" .gitlab-ci.yml .circleci/config.yml azure-pipelines.yml bitbucket-pipelines.yml .travis.yml Jenkinsfile .buildkite; do
  [ -e "$f" ] && exit 0
done

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
  exit 0
fi

# The files as they are now (ignored files left out), as a git tree id, built in a scratch index.
tree=""
if idx=$(mktemp "${TMPDIR:-/tmp}/claude-commit-gate-index.XXXXXX"); then
  trap 'rm -f "$idx"' EXIT
  cp "$(git rev-parse --git-path index)" "$idx" 2>/dev/null || rm -f "$idx"   # a copy lets git skip unchanged files
  tree=$(GIT_INDEX_FILE="$idx" git add -A 2>/dev/null && GIT_INDEX_FILE="$idx" git write-tree 2>/dev/null) || tree=""
fi
[ -n "$tree" ] && [ "$(cat "$state.pass" 2>/dev/null)" = "$tree" ] && exit 0

if out=$(bash -c "$check" 2>&1); then
  rm -f "$state.fails"
  [ -n "$tree" ] && echo "$tree" >"$state.pass"
  exit 0
fi

if [ "$mode" = observe ]; then block check-failed ""; exit 0; fi
n=$(( $(cat "$state.fails" 2>/dev/null || echo 0) + 1 ))
echo "$n" >"$state.fails"
if [ "$n" -ge 3 ]; then
  next="This check has failed $n times in a row. Stop. Do not push. Report to the user what fails, what you tried, and your best guess at the cause."
else
  next="Fix the code, not the tests or checks. If the failure is flaky or was already failing before your change, follow AGENTS.md Verification 5."
fi
block check-failed "Push blocked: \`$check\` failed (attempt $n).
--- last 60 lines ---
$(tail -n 60 <<<"$out")
---
$next"
