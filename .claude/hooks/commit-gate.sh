#!/usr/bin/env bash
# Claude Code hook, registered for three events:
#   PostToolUse (matcher Skill) and UserPromptSubmit: note that this session ran the verify
#     or code-review skill (Claude called it, or the user typed /verify).
#   PreToolUse (matcher Bash): gate `git commit` and `git push`.
# Exit 0 allows. Exit 2 blocks, and stderr is shown to Claude.
#
# Each is judged in the checkout it is made in: the folder the command starts in, with the moves
# before it and git's own -C. CLAUDE_PROJECT_DIR does not say: in a session of a git worktree it
# is the main checkout. A command whose checkout can only be known once it runs is blocked.
#
# git commit is allowed when any of these hold:
#   - this session ran the verify or code-review skill
#   - the commit message says why it was not verified: `Verified: none (<reason>)`, in the
#     command or in a file it gives with -F
#   - a session that ran the verify skill committed on this branch before, and the work since
#     is not on the default branch yet: the task was verified, and this is a fix or an update to
#     its PR, maybe in a new session. Such commits are noted in .git/lim-verified-branches.
#   - a merge, cherry-pick, revert, or rebase is in progress
#   - commits were already blocked twice in this session, so Claude does not get stuck
# It is always blocked while .claude/verify.sh has uncommitted changes, a deletion included.
#
# git push runs the repo's checks first, only in a repo without CI config (with CI, CI checks
# the PR). The checks are .claude/verify.sh, or a guess from the project files. A failing check
# blocks the push every time. A tree that already passed in this session is not checked again.
#
# LIM_COMMIT_GATE=off|observe|enforce (default enforce). observe writes what would have been
# blocked to .git/lim-commit-gate.log and allows it.

set -uo pipefail

mode=${LIM_COMMIT_GATE:-enforce}
[ "$mode" = off ] && exit 0

# The hook runs on every Bash call and every prompt, and most concern neither git nor the verify
# skill, so it leaves before starting any other program. JSON does not escape letters, so these
# words are in the input as written whenever they are in a field.
IFS= read -r -d '' input || true
case $input in *git*|*verify*|*code-review*) ;; *) exit 0 ;; esac

# Without jq the hook cannot read its input, so it stays out of the way.
command -v jq >/dev/null 2>&1 || exit 0

# Every field in one jq run, each ended by a NUL (taken out of the fields themselves). If jq
# cannot read the input, the fields stay empty and the hook allows the call, as before.
event='' session='' text='' cwd=''
{ IFS= read -r -d '' event; IFS= read -r -d '' session; IFS= read -r -d '' text; IFS= read -r -d '' cwd; } < <(
  jq -j '[.hook_event_name // "PreToolUse", .session_id // "default",
          (if .hook_event_name == "PostToolUse" then .tool_input.skill
           elif .hook_event_name == "UserPromptSubmit" then .prompt else .tool_input.command end) // "",
          .cwd // ""] | map(tostring | split("\u0000") | join("") + "\u0000") | add' <<<"$input")
while [[ $text == *$'\n' ]]; do text=${text%$'\n'}; done
while [[ $cwd == *$'\n' ]]; do cwd=${cwd%$'\n'}; done
state="${TMPDIR:-/tmp}/claude-commit-gate-${session//[^A-Za-z0-9_-]/}"

skill_re='(^|:)(verify|code-review)$'
typed_re=$'(^|\n)[[:space:]]*/([^[:space:]]*:)?(verify|code-review)([[:space:]]|$)'
case $event in
  PostToolUse)
    [[ $text =~ $skill_re ]] && touch "$state.verified"
    exit 0 ;;
  UserPromptSubmit)   # print nothing: stdout here would be added to Claude's context
    [[ $text =~ $typed_re ]] && touch "$state.verified"
    exit 0 ;;
  PreToolUse) ;;
  *) exit 0 ;;
esac

# A call is git followed by commit or push, so a command without both words has none.
cmd=$text
[[ $cmd == *git* && ( $cmd == *commit* || $cmd == *push* ) ]] || exit 0

# The command as it is read for calls and moves: without the bodies of heredocs that are a commit
# message (given to git commit -F -, or in $(cat <<...)) and the values of -m, so that the words
# of a message are not taken for commands. A heredoc given to anything else, a shell among them,
# is read as it is, and so is one whose end never comes. The message itself is read from the
# command as written.
scan=$(awk '
  skip {
    t = $0; if (strip) sub(/^\t+/, "", t)
    if (t == delim) { skip = 0; held = ""; next }
    held = held $0 "\n"; next
  }
  { print }
  ($0 ~ /\$\([ \t]*cat[ \t]*<</ || $0 ~ /(-[A-Za-z]*F[ \t]*-|--file[= \t]+-)([ \t;&|)<]|$)/) &&
  match($0, /(^|[^<])<<-?[ \t]*["\047]?[^ \t;&|<>()"\047]+["\047]?/) {
    d = substr($0, RSTART, RLENGTH); sub(/^[^<]/, "", d)
    strip = (d ~ /^<<-/); sub(/^<<-?[ \t]*/, "", d); gsub(/["\047]/, "", d); delim = d; skip = 1; held = ""
  }
  END { if (skip) printf "%s", held }' <<<"$cmd")
# A value starts on the line of its -m: the next line is the next command.
opt="(^|[[:space:]])(-[[:alpha:]]*m|--message)([=[:blank:]]*)"
msgval="(\"[^\"]*\"|'[^']*'|[^[:space:];&|()\"']+)"
# Text up to a match is cut by its length: ${rest#*"$m"} takes time squared in the length of a
# long command, such as one that writes a file with a heredoc.
rest=$scan scan=""
while [[ $rest =~ $opt$msgval ]]; do
  pre=${rest%%"${BASH_REMATCH[0]}"*}
  scan+=$pre${BASH_REMATCH[1]}${BASH_REMATCH[2]}${BASH_REMATCH[3]}x
  rest=${rest:${#pre}+${#BASH_REMATCH[0]}}
done
scan+=$rest
# A quoted string that mentions git is text, as in grep "git commit" or echo '... git push',
# unless something runs it: a shell's or su's -c before it in the same command (bash -c, sh -ec,
# su - me -c), eval or exec, or a $( or backquote inside double quotes. Its text is left out, so
# it is not taken for a call, nor does it use up the blocks after which a commit is let through.
scan=$(awk '
  { s = s (NR > 1 ? "\n" : "") $0 }
  END {
    out = ""; seg = ""; rest = s
    while (match(rest, /\\.|["\047]/)) {
      pre = substr(rest, 1, RSTART - 1); c = substr(rest, RSTART, RLENGTH); rest = substr(rest, RSTART + RLENGTH)
      if (length(c) > 1) { out = out pre c; seg = seg pre c; continue }
      out = out pre; seg = seg pre
      if (match(seg, /.*[;&|(){}`\n]/)) seg = substr(seg, RLENGTH + 1)
      body = ""; r = rest; closed = 0; end = (c == "\"") ? "\\\\.|\"" : "\047"
      while (match(r, end)) {
        if (RLENGTH > 1) { body = body substr(r, 1, RSTART + 1); r = substr(r, RSTART + 2); continue }
        body = body substr(r, 1, RSTART - 1); r = substr(r, RSTART + 1); closed = 1; break
      }
      if (!closed) { out = out c rest; rest = ""; break }
      rest = r
      run = seg ~ /(^|[ \t\/])((ba|z|da|k|fi)?sh|su)([ \t]+[^ \t]+)*[ \t]+-[A-Za-z]*c[ \t]*$/ || seg ~ /(^|[ \t])(eval|exec)[ \t]*$/ ||
            (c == "\"" && (index(body, "$(") || index(body, "`")))
      piece = (run || body !~ /(^|[^A-Za-z0-9_-])git[ \t]/) ? c body c : c c
      out = out piece; seg = seg piece
    }
    printf "%s", out rest
  }' <<<"$scan")

# A call of git $1: also `/usr/bin/git $1`, `git -C "/a b" -c k="v w" $1`, `... && git $1`,
# `bash -c 'git $1'`, `(git $1)` and `git commit;git push`, but not `git commit-tree`. Group 2 is
# the call itself, from git to $1. A word may hold quoted spaces, and a quote with no partner, as
# in O\'Brien, is an ordinary character.
piece="(\"[^\"]*\"|'[^']*'|\\\\.|[^[:space:]])"
value="(\"[^\"]*\"|'[^']*'|\\\\.|[^-[:space:]])$piece*"
gitcall() { printf '%s' "([^[:space:];&|()\"'\`]*/)?git[[:space:]]+([\"']?-${piece}+[[:space:]]+(${value}[[:space:]]+)?)*$1"; }
commit_re="(^|[;&|(\"'\`\\[:space:]])($(gitcall commit))([;&|)<>\"'\`[:space:]]|\$)"
push_re="(^|[;&|(\"'\`\\[:space:]])($(gitcall push))([;&|)<>\"'\`[:space:]]|\$)"
commits=; pushes=
grep -Eq "$commit_re" <<<"$scan" && commits=1
grep -Eq "$push_re" <<<"$scan" && pushes=1
[ -n "$commits$pushes" ] || exit 0

log_in=""   # the checkout whose .git the observe log goes to
block() {  # $1: reason for the log, $2: message for Claude. In observe mode, log and carry on.
  if [ "$mode" = observe ]; then
    local log
    log=$(common "${log_in:-${CLAUDE_PROJECT_DIR:-$cwd}}") || return 0
    printf '%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$session" "$1" "$(head -c 200 <<<"$cmd" | tr '\t\n' '  ')" \
      >>"$log/lim-commit-gate.log"
    return 0
  fi
  echo "$2" >&2
  exit 2
}

# The .git folder that checkout $1 shares with its worktrees.
common() { (cd "$1" 2>/dev/null && cd "$(git rev-parse --git-common-dir 2>/dev/null)" && pwd); }

unknown() {
  block unknown-checkout "Blocked: cannot tell which checkout \`$1\` leads to, so it cannot be checked. Name an existing folder in full: git -C /path/to/checkout commit. If this is only text in the command, such as a commit message, pass it in a file."
  exit 0
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

# What the command does before its last commit or push.
before=""
for kind in ${commits:+commit} ${pushes:+push}; do
  b=${scan%git*"$kind"*}
  [ ${#b} -gt ${#before} ] && before=$b
done

# A cd that opens the command moves all that follows, unless a lone & sends it to the background.
word="(\"[^\"]*\"|'[^']*'|[^[:space:];&|()\"'])+"
opening="^[[:space:]]*cd[[:space:]]+($word)[[:space:]]*(&&|;|"$'\n'")"
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

# Each call in the command. A match ends on the character after the subcommand, and that
# character can also open the next one, as in `git commit;git -C dir commit`, so it is put back.
# A call met before is not looked at again.
calls=() kinds=() commit_args=""
argpiece="(\"[^\"]*\"|'[^']*'|\\\\.|[^[:space:];&|()<>])"
collect() {  # $1: kind, $2: its pattern
  local rest=$scan seen=$'\n' whole call pre
  while [[ $rest =~ $2 ]]; do
    whole=${BASH_REMATCH[0]} call=${BASH_REMATCH[2]}
    if [[ $seen != *$'\n'"$call"$'\n'* ]]; then
      calls+=("$call") kinds+=("$1")
      seen+=$call$'\n'
    fi
    pre=${rest%%"$whole"*} rest=${rest:${#pre}+${#whole}}
    # What git commit is given, up to the end of its command or its line.
    if [ "$1" = commit ] && [[ ${whole: -1} == [[:blank:]] && $rest =~ ^($argpiece|[[:blank:]])* ]]; then
      commit_args+=" ${BASH_REMATCH[0]}"
    fi
    rest=${whole: -1}$rest
  done
}
[ -z "$commits" ] || collect commit "$commit_re"
[ -z "$pushes" ] || collect push "$push_re"
# grep saw a call, so not finding it here must not let it through.
[ ${#calls[@]} -gt 0 ] || unknown "$cmd"

# The checkouts each kind of call is made in, and the folders git commit starts in.
commit_tops=() push_tops=() commit_dirs=()
add() {  # $1: kind, $2: checkout
  local known
  if [ "$1" = commit ]; then
    for known in ${commit_tops[@]+"${commit_tops[@]}"}; do [ "$known" = "$2" ] && return; done
    commit_tops+=("$2")
  else
    for known in ${push_tops[@]+"${push_tops[@]}"}; do [ "$known" = "$2" ] && return; done
    push_tops+=("$2")
  fi
}
for ((c = 0; c < ${#calls[@]}; c++)); do
  call=${calls[c]}
  dirs=("${here[@]}")
  # Split the call into words, keeping quoted spaces in their word.
  words=()
  while [[ $call =~ ^[[:space:]]*($piece+) ]]; do
    words+=("${BASH_REMATCH[1]}")
    call=${call#"${BASH_REMATCH[0]}"}
  done
  for ((i = 1; i < ${#words[@]}; i++)); do
    # Quotes do not change what git reads, and its config keys ignore case.
    bare=${words[i]//[\"\']/}
    case $(tr '[:upper:]' '[:lower:]' <<<"$bare") in
      --git-dir* | --work-tree* | *core.worktree=*) unknown "git ${words[i]}" ;;
    esac
    case $bare in
      -C)
        i=$((i + 1))
        next=()
        for dir in "${dirs[@]}"; do
          to=$(folder -P "$dir" "${words[i]:-}") && next+=("$to")
        done
        [ ${#next[@]} -gt 0 ] || unknown "git -C ${words[i]:-}"
        dirs=("${next[@]}")
        ;;
    esac
  done
  for dir in "${dirs[@]}"; do
    [ "${kinds[c]}" = commit ] && commit_dirs+=("$dir")
    top=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) || continue
    add "${kinds[c]}" "$top"
  done
done

# Outside any checkout, the project's own is checked: leaving it must not skip the gate.
if top=$(git -C "${CLAUDE_PROJECT_DIR:-$start}" rev-parse --show-toplevel 2>/dev/null); then
  [ -z "$commits" ] || [ ${#commit_tops[@]} -gt 0 ] || add commit "$top"
  [ -z "$pushes" ] || [ ${#push_tops[@]} -gt 0 ] || add push "$top"
fi

# Whether a merge, cherry-pick, revert, or rebase is in progress in checkout $1.
in_progress() {
  (cd "$1" && for p in MERGE_HEAD CHERRY_PICK_HEAD REVERT_HEAD rebase-merge rebase-apply; do
    [ -e "$(git rev-parse --git-path "$p")" ] && exit 0
  done; exit 1)
}

# The default branch as checkout $1 knows it: the remote's, which a stale local one is not.
default_branch() {
  local b
  b=$(git -C "$1" symbolic-ref -q --short refs/remotes/origin/HEAD 2>/dev/null) && { echo "$b"; return; }
  for b in origin/main origin/master main master; do
    git -C "$1" rev-parse -q --verify "$b^{commit}" >/dev/null 2>&1 && { echo "$b"; return; }
  done
}

# Note that a session that ran the verify skill commits on the branch of checkout $1, from its
# current commit.
remember() {
  local branch head line log
  branch=$(git -C "$1" symbolic-ref -q --short HEAD 2>/dev/null) || return 0
  head=$(git -C "$1" rev-parse -q --verify 'HEAD^{commit}' 2>/dev/null) || return 0
  log=$(common "$1") || return 0
  line=$(printf '%s\t%s' "$branch" "$head")
  grep -qxF "$line" "$log/lim-verified-branches" 2>/dev/null || printf '%s\n' "$line" >>"$log/lim-verified-branches"
}

# Whether a verified session committed on the branch of checkout $1 before, and that commit is
# among the branch's work the default branch does not have yet: a commit in it whose parent is
# the noted one. A branch made again under the same name builds on the default branch instead.
verified_before() {
  local branch head log base b from
  branch=$(git -C "$1" symbolic-ref -q --short HEAD 2>/dev/null) || return 1
  head=$(git -C "$1" rev-parse -q --verify 'HEAD^{commit}' 2>/dev/null) || return 1
  log=$(common "$1") || return 1
  [ -f "$log/lim-verified-branches" ] || return 1
  base=$(default_branch "$1")
  while IFS=$'\t' read -r b from; do
    [ "$b" = "$branch" ] || continue
    git -C "$1" rev-list --parents "$head" "^$from" ${base:+"^$base"} 2>/dev/null |
      awk -v f="$from" '{ for (i = 2; i <= NF; i++) if ($i == f) found = 1 } END { exit !found }' && return 0
  done <"$log/lim-verified-branches"
  return 1
}

# Whether the message says why the commit was not verified: in the command, or in a file it
# gives with -F or --file, which git reads from the folder it commits in.
none_re='Verified:[[:space:]]*none[[:space:]]*\([^)]*[^)[:space:]][^)]*\)'
said_none() {
  local f d p
  grep -Eq "$none_re" <<<"$cmd" && return 0
  while IFS= read -r f; do
    f=${f#\"} f=${f%\"} f=${f#\'} f=${f%\'}
    [ -n "$f" ] && [ "$f" != - ] || continue
    for d in ${commit_dirs[@]+"${commit_dirs[@]}"}; do
      case $f in /*) p=$f ;; *) p=$d/$f ;; esac
      [ -f "$p" ] && grep -Eq "$none_re" "$p" && return 0
    done
  done < <(grep -Eo "(^|[[:space:]])(-[[:alpha:]]*F[[:space:]]*|--file[=[:space:]]+)$piece+" <<<"$commit_args" | sed -E 's/^[[:space:]]*(-[[:alpha:]]*F[[:space:]]*|--file[=[:space:]]+)//')
  return 1
}

commit_allowed() {
  local top n all=1
  if [ -e "$state.verified" ]; then
    for top in "${commit_tops[@]}"; do remember "$top"; done
    return 0
  fi
  said_none && return 0
  for top in "${commit_tops[@]}"; do
    in_progress "$top" || verified_before "$top" || { all=""; break; }
  done
  [ -n "$all" ] && return 0
  [ "$mode" = observe ] && return 1
  n=$(cat "$state.blocks" 2>/dev/null || echo 0)
  if [ "$n" -ge 2 ]; then rm -f "$state.blocks"; return 0; fi
  echo $((n + 1)) >"$state.blocks"
  return 1
}

if [ ${#commit_tops[@]} -gt 0 ]; then
  for top in "${commit_tops[@]}"; do
    log_in=$top
    # A change must not edit the check that judges it, nor delete or rename it.
    if [ -n "$(git -C "$top" status --porcelain -- .claude/verify.sh)" ]; then
      block verify.sh-changed "Commit blocked: .claude/verify.sh has uncommitted changes in $top. Do not change the checks that judge your change. Ask the user to review and commit verify.sh themselves."
    fi
  done
  commit_allowed || block no-verify "Commit blocked: this session has not run the verify skill. Run it before the first commit (AGENTS.md Order of work 3). If this commit does not need verifying, end its message with \`Verified: none (<reason>)\`."
fi

# git push: with CI, CI checks the PR.
for top in ${push_tops[@]+"${push_tops[@]}"}; do
  cd "$top" || continue
  log_in=$top
  set -- .github/workflows/*.yml .github/workflows/*.yaml
  ci=""
  for f in "$@" .gitlab-ci.yml .circleci/config.yml azure-pipelines.yml bitbucket-pipelines.yml .travis.yml Jenkinsfile .buildkite; do
    [ -e "$f" ] && { ci=1; break; }
  done
  [ -z "$ci" ] || continue

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

  # The files as they are now (ignored files left out), as a git tree id, built in a scratch index.
  tree=""
  if idx=$(mktemp "${TMPDIR:-/tmp}/claude-commit-gate-index.XXXXXX"); then
    cp "$(git rev-parse --git-path index)" "$idx" 2>/dev/null || rm -f "$idx"   # a copy lets git skip unchanged files
    tree=$(GIT_INDEX_FILE="$idx" git add -A 2>/dev/null && GIT_INDEX_FILE="$idx" git write-tree 2>/dev/null) || tree=""
    rm -f "$idx"
  fi
  [ -n "$tree" ] && [ "$(cat "$state.pass" 2>/dev/null)" = "$tree" ] && continue

  if out=$(bash -c "$check" 2>&1); then
    rm -f "$state.fails"
    [ -n "$tree" ] && echo "$tree" >"$state.pass"
    continue
  fi

  if [ "$mode" = observe ]; then block check-failed ""; continue; fi
  n=$(( $(cat "$state.fails" 2>/dev/null || echo 0) + 1 ))
  echo "$n" >"$state.fails"
  if [ "$n" -ge 3 ]; then
    next="This check has failed $n times in a row. Stop. Do not push. Report to the user what fails, what you tried, and your best guess at the cause."
  else
    next="Fix the code, not the tests or checks. If the failure is flaky or was already failing before your change, follow AGENTS.md Verification 5."
  fi
  block check-failed "Push blocked: \`$check\` failed in $top (attempt $n).
--- last 60 lines ---
$(tail -n 60 <<<"$out")
---
$next"
done
exit 0
