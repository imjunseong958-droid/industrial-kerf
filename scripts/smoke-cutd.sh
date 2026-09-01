#!/bin/sh
set -eu

project_dir=$(mktemp -d "${TMPDIR:-/tmp}/cutd-smoke.XXXXXX")
trap 'rm -rf "$project_dir"' EXIT HUP INT TERM

first_output=$(
  printf '%s\n' \
    '{"protocolVersion":1,"id":"state-0","operation":"get_state"}' \
    '{"protocolVersion":1,"id":"bad-key","operation":"apply","actor":"ui","idempotencyKey":"","baseRevision":0,"command":{"type":"set_metadata","patch":{"bad":true}}}' \
    '{"protocolVersion":1,"id":"apply-1","operation":"apply","actor":"ui","idempotencyKey":"smoke-1","baseRevision":0,"command":{"type":"set_metadata","patch":{"smoke":true}}}' \
    | cargo run -q -p cut-runtime --bin cutd -- --stdio --dir "$project_dir"
)

printf '%s\n' "$first_output" | grep '"id":"state-0"' >/dev/null
printf '%s\n' "$first_output" | grep '"id":"bad-key"' | grep '"ok":false' >/dev/null
printf '%s\n' "$first_output" | grep '"id":"apply-1"' | grep '"revision":1' >/dev/null

proposal_output=$(
  printf '%s\n' \
    '{"protocolVersion":1,"id":"proposal","operation":"stage_proposal","requestKey":"agent-1","baseRevision":1,"transactions":[{"idempotencyKey":"agent-1","commands":[{"type":"set_metadata","patch":{"approved":true}}]}]}' \
    | cargo run -q -p cut-runtime --bin cutd -- --stdio --dir "$project_dir"
)

printf '%s\n' "$proposal_output" | grep '"proposalId"' | grep '"baseRevision":1' >/dev/null

restart_output=$(
  printf '%s\n' \
    '{"protocolVersion":1,"id":"state-reopen","operation":"get_state"}' \
    | cargo run -q -p cut-runtime --bin cutd -- --stdio --dir "$project_dir"
)

printf '%s\n' "$restart_output" | grep '"id":"state-reopen"' | grep '"revision":1' | grep '"smoke":true' >/dev/null

echo 'cutd smoke passed'
