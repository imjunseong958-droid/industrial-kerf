# Agentic 미디어 관찰 실행계획

상태: 단계 2~5 구현 완료 및 workflow 실행 경계 검증 완료 (로컬 기반; 외부 생성 provider 미연결)
작성일: 2026-09-06

이 문서는 Google의 agentic video understanding과 최신 Diffusion Studio
editor의 관찰·검증·에셋 관리 흐름에서 차용할 범위와 Cut의 다음 작업을
정리한다. 프로젝트의 정본은 계속 `cutd`가 소유하는 typed `Project`이며,
agent의 편집은 기존 `timeline.apply` proposal 경계를 통과한다.

## 차용할 제품 원칙

- Google의 방식에서 **coarse → targeted evidence loop**를 차용한다. 먼저
  저렴한 filmstrip 같은 희소 관찰로 후보 구간을 좁히고, 그 다음 필요한
  timestamp frame, 오디오, transcript를 선택한 범위에서 요청한다. 외부
  Gemini 호출이나 API 키는 이번 작업에 포함하지 않는다.
- Diffusion Studio의 `probe`, `filmstrip`, `waveform`, `transcribe`,
  `listen`, `context`, `check`, `capture`가 분리된 에이전트 흐름과
  프로젝트 library에 생성 결과를 캐시하는 방식을 차용한다. Cut에서는
  결과를 `.cut/cache/derived` 아래의 재생성 가능한 artifact로 유지한다.
- 관찰 근거에는 source identity, 시간 구간, 생성된 artifact 경로와 cache
  상태를 남긴다. 근거와 편집 명령을 canonical project 데이터에 섞지 않는다.

참고 자료: [Google agentic video research](/tmp/cut-google-research.md),
[Cut 아키텍처 감사](/tmp/cut-architecture-plan-audit.md),
[Gemini agentic video understanding](https://ai.google.dev/gemini-api/docs/video-understanding#agentic-video-understanding),
[Diffusion Studio CLI reference](https://github.com/diffusionstudio/editor/blob/main/reference/README.md).

## 현재 기능과 확인된 gap

현재 Cut은 이미 다음 기능을 갖고 있다.

- `project.outline`과 `timeline.query`가 source/track/clip의 구조와 시간을
  읽는다.
- `media.probe`가 info, silence, scenes, exact frame, filmstrip/contact
  sheet, waveform, bounded local transcription/listen을 각각 제공한다.
- `timeline.check`가 구조·소스·컴파일 가능성을 검사하고,
  `timeline.capture`가 offline compiler로 편집 결과의 bounded frame 또는
  filmstrip을 만든다. `proposal.capture`는 pending proposal의 before/after
  결과를 캡처한다.
- derivative는 source identity와 옵션을 포함한 content-addressed cache에
  저장되며, transcript/listen은 선택된 범위를 별도로 키로 삼는다.

부족한 연결은 agent가 `project.outline`에서 얻은 source를 다시 경로로
해석하고, 여러 `media.probe` 호출의 동일 시간 구간을 직접 조합해야 한다는
점이다. 결과마다 range와 provenance가 흩어져 coarse 관찰에서 targeted
관찰로 넘어갈 때 근거를 잃기 쉽다. 또한 기존 source ID를 `media.probe`
`src`에 바로 전달할 수 없었다.

## 1차 구현: bounded observation envelope

기존 도구를 복제하지 않고 `media.probe`의 action 모델에 `observe`를
추가한다. 입력은 기존 `src`/`opts` 형태를 유지한다.

```json
{
  "action": "observe",
  "src": "source-1",
  "opts": {
    "start": 12.0,
    "duration": 8.0,
    "evidence": ["filmstrip", "frames", "transcript", "listen"],
    "frames": 6,
    "includeBase64": false
  }
}
```

`start`와 `end` 또는 `duration`은 필수이고, 한 요청은 최대 120초로
제한한다. `evidence`를 생략하면 가장 저렴한 기본 관찰인 `filmstrip`을
선택한다. 선택 가능한 modality는 `frames`, `filmstrip`, `waveform`,
`transcript`, `listen`이며 각 항목은 기존 cache/sidecar 구현을 호출한다.
`listen`은 bounded WAV 경로와 transcript를 함께 반환한다. `waveform`은
현재 구현이 source 전체를 계산하므로 envelope 안에 `scope: "source"`를
명시한다.

반환 envelope는 `{ ref, path }` 형태의 `source`, `media`, `range`, 실제 요청한 frame `times`,
그리고 `{ kind, data }` 형태의 evidence 목록을 포함한다. 경로와 metadata는
기본값으로 반환하고, 이미지 base64는 명시적으로 요청할 때만 포함한다.
각 artifact의 `key`와 `cached` 값은 기존 derivative contract를 그대로
노출한다. source ID 입력은 현재 Project의 source path로 안전하게 해석한다.

이 action은 read-only이며 project revision을 바꾸지 않는다. agent가 편집을
제안할 때는 기존 `timeline.apply`를 사용하고, 결과 확인은
`timeline.check` → `timeline.capture` 또는 `proposal.capture` 순서를
따른다. 따라서 관찰 contract가 canonical state, typed command, proposal
수명주기를 우회하지 않는다.

## 완료 기준

- 동일한 bounded source range를 지정한 observe 결과에 source provenance,
  range, evidence kind가 함께 들어간다.
- `filmstrip`/`frames`는 선택 범위에서 생성된 timestamp와 cache metadata를
  반환한다.
- `transcript`/`listen`은 기존 local whisper 정책과 범위 제한을 지키며,
  provider backend를 암묵적으로 사용하지 않는다.
- 반복 observe는 기존 cache를 재사용하고, source 교체 시 기존 identity
  invalidation을 따른다.
- `includeBase64` 기본값은 false이고 canonical Project에는 media bytes,
  transcript, derivative path가 기록되지 않는다.
- observe → `timeline.apply` proposal → `timeline.check` →
  `timeline.capture`/`proposal.capture` 흐름을 API 키와 유료 호출 없이
  로컬 계약 테스트로 검증할 수 있다.

### 단계 2: agent run trace — 구현 완료

`AgentSession::run_with_events`가 bounded text/tool 이벤트를 전달하고,
`media.probe(action: observe)` 결과의 phase(`coarse`/`targeted`), range, evidence, artifact
경로를 요약해 Qt 활동 로그에 표시한다. provider payload, frame base64,
credential은 이벤트에 들어가지 않는다. 실패와 중단은 별도 상태로 표시한다.

### 단계 3: 범위 waveform — 구현 완료

`cached_waveform_range`가 선택한 source 범위를 cache key에 포함하고 decoder와
waveform image 모두 `-ss`/`-t` 범위를 사용한다. 기존 full-source waveform도
`start: 0`, `end: duration`인 같은 contract로 반환된다.

### 단계 4: typed observation reference와 평가 IR — 구현 완료

observe envelope는 source identity, source/clip 시간과 artifact key를 가진
`observationRef`를 반환한다. agent가 `timeline.apply`에 이를 전달하면
`cutd`의 pending proposal에 reference가 저장되어 restart 후에도 proposal
impact와 함께 검토할 수 있다. `cut-core::evaluation::EvaluationPlan`은
timeline/source 시간, speed, enabled/muted, layer 순서를 immutable typed
계약으로 제공하고 offline compiler와 native playback이 이를 검증/보유한다.
고급 effect와 완전한 frame parity는 아직 capability matrix의 범위다. typed
reference에는 selection index, rationale, optional coarse reference도
보존된다. project source에 묶인 observation은 revision과 현재 path/stat을
staging 및 acceptance에서 다시 확인하고, project 밖 raw source는 revision
없이 파일 identity만 확인한다.

### 단계 5: generated asset cache와 provenance — 구현 완료

`media.asset`은 이미 생성된 로컬 파일만 content fingerprint와 request
fingerprint로 `.cut/cache/assets.json`에 등록한다. manifest RMW는 전용 lock을
잡고 기록 파일을 sync한 뒤 atomic rename하며, 동시 호출의 provenance 레코드를
보존한다. `import: true`는 canonical `media_import` 직접 호출 대신 runtime의
`stage_import_proposal`을 사용한다. 따라서 import와 `replaceClipId` 교체가
하나의 typed proposal에 들어가며 proposal 수락 전 canonical Project는
변하지 않는다. request/content identity에서 안정적인 operation key와 import
ID를 만들기 때문에 반복 호출은 같은 proposal로 수렴한다. 외부 provider를
호출하지 않으며 파일이 없는 생성 결과를 성공으로 기록하지 않는다. 실제
영상 생성은 provider 설정과 명시적 비용/동의 정책이 갖춰진 뒤에 별도 작업으로
남긴다. 같은 request fingerprint에 다른 content hash를 다시 묶을 수 없으며,
hash 계산 뒤 staging 직전에 파일 hash를 재검증한다.

### 연결된 사용 흐름

agent 요청은 먼저 `project.outline`과 coarse `media.probe(action: observe)`를 호출해 근거를
모은 뒤, agent가 선택한 복수 source ranges와 각 rationale을
`workflow.roughcut.selections`로 전달한다. workflow는 선택된 각 범위를
targeted observe하고 add/update clip typed commands를 하나의 proposal로
묶은 뒤 `timeline.check(proposalId)`로 후보 구조를 검증하고
`proposal.capture`를 실행한다. `duration`이 있으면 speed를 반영한 선택
timeline 총합의 상한으로 적용하고 컷을 자동으로 줄이지 않는다. capture는
제안된 timeline 범위를 우선 포함하며, 범위가 120초보다 흩어지면 대표 범위와
coverage를 반환한다. selections가 없으면 `planning_needed`를
반환하며 첫 video clip/source를 자동 선택하지 않는다. 후보가 invalid이면
proposal을 reviewable 상태로 성공 처리하지 않고 거절하며 canonical state는
그대로다. 반환 상태가 `awaiting_review`일 때만 사람이 proposal을 확인하고
기존 `AcceptProposal` 경계를 통해 적용한다. 각 observation range는 120초를
넘을 수 없고 `maxRetries`는 2 이하이며, agent에는 direct
`media.fetch`/`media.import`/undo/redo가 노출되지 않는다. native local
artifact가 없으면 `media.asset`은 파일 부재 오류를 반환한다.
