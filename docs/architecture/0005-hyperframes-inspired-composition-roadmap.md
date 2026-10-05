# HyperFrames에서 영감을 받은 Cut Composition/Template 로드맵

> 상태: ADR 0006 phase 5에서 `composition.*` MCP 도구와 구현을 삭제했습니다. 에이전트 하네스는 `look`, `edit`, `render`만 제공합니다. 아래 내용은 기록으로 남깁니다.
>
> 조사 결과와 첫 구현 계약을 정리한 문서입니다. 초기 범위는 manifest/check였고, 현재는 최소한의 local catalog와 외부 renderer bridge 수직 슬라이스까지 연결했습니다.

## 실행 상태

- 기존 구현 변경사항은 `cbeaeab feat(mcp): inspect HTML motion compositions`로 커밋하고 `origin/main`에 푸시했습니다.
- CLI 설치 모드로 `/Users/limjunseong/.local/bin`에 `lcut`, `cutd`, `cut-mcp`를 설치했습니다.
- `composition.check`에 schema v1 manifest, typed slots, HTML variable binding,
  sparse instance override 검증을 추가했습니다. 이 검사는 read-only이며
  canonical Project/revision을 변경하지 않습니다.
- 후속 최소 통합으로 `composition.catalog`, `composition.preview`,
  `composition.render`를 Core/Full profile에 등록하고, namespace·agent
  discovery·라우팅·프로토콜 fixture를 연결했습니다. catalog는 local
  sidecar discovery만 수행하고, preview/render는 고정된 외부 JSON bridge와
  bounded snapshot/cache/provenance를 사용하며 canonical Project는 바꾸지
  않습니다.

## 조사 결론

HyperFrames의 핵심은 단순히 HTML로 영상을 만든다는 점이 아닙니다. 재사용 가능한 composition을 하나의 소스 오브 트루스로 두고, 미디어와 변수만 바꾸어 여러 결과물을 만들며, 같은 composition을 inspect/edit/play/render하는 구조가 핵심입니다. HyperFrames는 HTML, CSS, 미디어, seekable animation을 결정론적인 영상으로 바꾸는 런타임으로 설명됩니다. [HyperFrames README](https://github.com/heygen-com/hyperframes), [개발자 개요](https://github.com/heygen-com/hyperframes/blob/main/docs/developers/overview.mdx)

이 구조에서 중요한 개념은 다음과 같습니다.

- composition: 장면과 타이밍, 스타일, 애니메이션을 담은 재사용 가능한 영상 템플릿
- 안정적인 ID와 데이터 속성: 에이전트나 SDK가 특정 요소를 찾아 수정할 수 있는 편집 표면
- typed variables와 media slots: 텍스트, 색상, 숫자, boolean, enum, 이미지, 영상 등을 교체 가능한 입력으로 분리
- instance override: 원본 템플릿은 그대로 두고 프로젝트별 변경값만 저장
- deterministic rendering: 같은 composition, 입력, 프레임 조건에서 같은 결과를 보장
- preview/render 분리: 브라우저에서 확인하는 단계와 최종 영상 산출 단계를 구분

HyperFrames Launches 저장소는 각 하위 디렉터리를 독립적인 composition으로 다루며, `index.html`, `compositions/`, `assets/`, `renders/`, `STORYBOARD.md`, `meta.json` 같은 구성을 사용합니다. 또한 CLI로 preview와 render를 수행하고, 일부 템플릿은 동일한 장면 구조에서 텍스트·색상·creator clip 같은 값만 바꾸는 방식으로 재사용됩니다. [HyperFrames Launches](https://github.com/heygen-com/hyperframes-launches), [변수 사용 예시](https://raw.githubusercontent.com/heygen-com/hyperframes-launches/main/variables-launch/index.html), [scene-04 예시](https://raw.githubusercontent.com/heygen-com/hyperframes-launches/main/variables-launch/compositions/scene-04.html)

## 나중에 Cut에 적용할 작업 순서

### 1. Template manifest와 typed slots

먼저 HTML을 바로 편집하는 기능보다, 템플릿이 어떤 입력을 받는지 명시하는 manifest를 정의하는 것이 좋습니다.

예상 항목은 다음과 같습니다.

- template id와 version
- width, height, fps, duration 또는 duration 계산 방식
- 교체 가능한 텍스트·색상·숫자·enum 변수
- 영상·이미지·오디오 media slot
- 필요한 폰트와 외부 의존성
- preview/render 진입점
- 라이선스와 원본 asset 정보

현재 Cut의 첫 계약은 composition.check의 선택적 manifest 입력으로
고정했습니다. schemaVersion 1 manifest는 프로젝트 상대 경로의 JSON
sidecar이며, templateId, templateVersion, entry, 명시적인
output(width, height, fps, duration), slots를 가집니다. slot 타입은
text, color, number, boolean, enum, image, video, audio이고, HTML은
data-var-* 또는 data-var="slotId"로 slot을 참조합니다.

프로젝트별 sparse override는 별도 instance sidecar로 둡니다.

~~~json
{
  "schemaVersion": 1,
  "templateId": "product-launch-v1",
  "templateVersion": "1.0.0",
  "overrides": {
    "hookText": "새로운 제품 소개",
    "accentColor": "#7C5CFC",
    "creatorClip": "asset_123"
  }
}
~~~

composition.check는 manifest와 instance를 읽기 전용으로 검사하며,
template identity, HTML root output, typed override, 필수 slot, unknown
binding을 검증합니다. 이 sidecar와 override는 canonical Project나
revision에 기록되지 않습니다. media slot의 값은 경로가 아니라 asset id
형식만 허용하며, 실제 asset provenance 연결은 이후 render/proposal
단계에서 추가합니다.

이렇게 하면 “템플릿 하나를 만들고 클립만 계속 바꿔 쓰는” 사용 사례를 명시적인 계약으로 만들 수 있습니다. HyperFrames는 `<html>`의 composition variables와 `data-var-*` 속성을 통해 이 방향을 구현합니다. [Variables and media](https://github.com/heygen-com/hyperframes/blob/main/skills/hyperframes-core/references/variables-and-media.md)

### 2. 프로젝트별 instance override

원본 composition과 프로젝트별 입력을 분리합니다. 프로젝트에는 전체 HTML 복사본 대신 다음과 같은 sparse override만 저장하는 방식입니다.

```json
{
  "schemaVersion": 1,
  "templateId": "product-launch-v1",
  "templateVersion": "1.0.0",
  "overrides": {
    "hookText": "새로운 제품 소개",
    "accentColor": "#7C5CFC",
    "creatorClip": "asset_123"
  }
}
```

이 모델은 같은 템플릿으로 여러 영상을 만들면서도 원본 템플릿의 업데이트와 개별 결과물의 변경을 구분할 수 있게 합니다. HyperFrames SDK의 embedded override mode도 base composition과 인스턴스별 patch를 분리하는 모델을 사용합니다. [Embedded override mode](https://github.com/heygen-com/hyperframes/blob/main/docs/sdk/guides/embedded-override-mode.mdx)

### 3. Composition library/catalog

재사용이 핵심이므로 파일을 저장하는 것만으로는 부족합니다. Cut 안에 composition을 찾고 선택할 수 있는 catalog가 필요합니다.

필요한 메타데이터는 다음과 같습니다.

- 이름, 설명, thumbnail, tag
- aspect ratio와 출력 포맷
- 필요한 slot과 변수
- 호환되는 asset 종류
- 템플릿 버전과 변경 이력
- 폰트·미디어·서드파티 라이선스
- 검증 상태와 마지막 preview/render 결과

초기에는 로컬 파일과 `meta.json` 정도로 시작하고, 나중에 공유 가능한 템플릿 저장소로 확장할 수 있습니다. Launches 저장소의 디렉터리별 composition과 `meta.json` 구조가 참고점입니다. [Launches repository](https://github.com/heygen-com/hyperframes-launches)

### 4. 에이전트의 HTML/source 편집을 proposal로 제한

에이전트가 composition을 직접 덮어쓰게 하기보다 다음 흐름으로 제한합니다.

1. 에이전트가 목표와 변경 대상을 해석
2. 템플릿 또는 composition source에 대한 proposal/diff 생성
3. `composition.check`와 구조 검증 수행
4. preview 또는 frame capture로 결과 확인
5. 사용자가 승인하면 템플릿 버전 또는 인스턴스 override로 저장

이렇게 해야 HTML은 강력한 편집 표면이 되면서도, 승인 전 변경이 canonical Project를 오염시키지 않습니다.

### 5. preview/render bridge

실제 렌더러를 Cut의 canonical state에 직접 넣기보다 별도의 composition worker 또는 HyperFrames CLI/Producer와 연결하는 것이 적절합니다.

권장 흐름은 다음과 같습니다.

```text
template + asset bindings
        ↓
lint / check / preview
        ↓
deterministic render
        ↓
media.asset (provenance 포함)
        ↓
timeline에 넣는 proposal
        ↓
사용자 승인 후 canonical Project 반영
```

HyperFrames의 개발자 문서도 통합 표면을 CLI, SDK, Player, Producer, Engine으로 나누며, 대부분의 통합에는 CLI나 Producer부터 시작하고 Engine은 자체 캡처·인코딩 인프라가 필요할 때 선택하도록 안내합니다. [개발자 개요](https://github.com/heygen-com/hyperframes/blob/main/docs/developers/overview.mdx), [Engine](https://github.com/heygen-com/hyperframes/blob/main/docs/packages/engine.mdx), [Rendering guide](https://github.com/heygen-com/hyperframes/blob/main/docs/guides/rendering.mdx)

### 6. 결정론과 캐시

재사용 템플릿은 렌더링 캐시와 함께 설계해야 합니다. 캐시 키에는 최소한 다음을 포함해야 합니다.

- template id와 version
- 변수와 media binding의 정규화된 값
- 입력 asset fingerprint
- renderer/runtime version
- width, height, fps, output format

또한 특정 프레임을 seek했을 때 같은 결과가 나오는지, 임의의 프레임 순서로 캡처해도 결과가 유지되는지 검증해야 합니다. HyperFrames Frame Adapter 문서는 정확한 프레임 seek, 유한한 정수 프레임 수, idempotence, wall-clock·random·network 비의존성을 결정론의 조건으로 둡니다. [Frame Adapters](https://github.com/heygen-com/hyperframes/blob/main/docs/concepts/frame-adapters.mdx)

### 7. storyboard/review loop

`STORYBOARD.md` 같은 사람이 읽는 설명과 실제 composition source, preview frame, 최종 render artifact를 연결하면 에이전트와 사람이 결과를 함께 검토하기 쉬워집니다.

최종적으로는 다음을 한 묶음으로 남기는 방향이 좋습니다.

- 어떤 템플릿 버전을 사용했는지
- 어떤 asset과 변수로 렌더했는지
- 주요 장면의 frame capture/contact sheet
- 렌더 결과와 provenance
- 승인된 proposal과 적용된 canonical 변경

## 현재 Cut 구조와의 정합성

이 아이디어를 도입하더라도 Cut의 canonical 모델은 유지해야 합니다.

- `cutd`는 Project state, revision, history, idempotency, proposal, render job의 소유자로 남깁니다. [Cut README](../../README.md)
- HTML composition은 canonical Project를 대체하지 않고, 파생 composition source 또는 렌더 입력으로 취급합니다.
- media slot에는 기존 `media.asset`의 provenance와 asset id를 연결합니다.
- 관찰·검증 결과는 canonical state와 분리된 observation/derivative/cache로 둡니다. [Agentic media observation plan](0003-agentic-media-observation-plan.md)
- 적용은 기존의 selection → proposal → check/capture → approval 흐름을 따릅니다.

현재 추가된 composition 도구들은 이 방향의 보수적인 수직 슬라이스입니다.
`composition.check`는 읽기 전용으로 HTML과 sidecar를 정적 검사하고,
`composition.catalog`는 bounded local metadata를 찾습니다.
`composition.preview`/`composition.render`는 브라우저를 Cut에 내장하지 않고
관리자가 지정한 외부 worker에만 deterministic JSON contract로 연결합니다.

## 범위에 넣지 않을 것

당장은 다음을 구현 범위에 넣지 않는 것이 좋습니다.

- Cut 안에 Node/Chrome 렌더러를 바로 내장하는 일
- HTML composition을 Project의 새로운 canonical 모델로 바꾸는 일
- CDN/network에 의존하는 템플릿을 그대로 production render에 사용하는 일
- 승인 없이 에이전트가 원본 템플릿이나 타임라인을 직접 덮어쓰는 일
- 공유 템플릿 catalog, 완전한 renderer, asset pipeline을 한 번에 productionize하는 일

먼저 manifest·slot·override·검증 계약을 고정하고, 이후에는 외부 worker
계약, proposal/review, canonical timeline 연결을 단계적으로 확장합니다.

## 운영상 가드레일

- 템플릿 source와 canonical Project를 분리합니다.
- 입력 asset은 경로가 아니라 검증된 asset id와 provenance로 바인딩합니다.
- 렌더 시 외부 CDN과 임의 network 요청을 제거하거나 명시적으로 허용 목록에 둡니다.
- preview와 최종 render 사이의 변수·미디어 바인딩 parity를 테스트합니다.
- 성공적인 프로세스 종료만 믿지 말고 최종 프레임과 출력 artifact를 확인합니다.
- 템플릿에 포함된 폰트·미디어·브랜드·서드파티 asset의 라이선스를 별도로 추적합니다. Launches 저장소도 소스 코드와 번들 미디어/폰트의 권리를 구분해 안내합니다. [Launches license note](https://github.com/heygen-com/hyperframes-launches)

HyperFrames의 공개 이슈에는 preview와 render 사이에서 variable override가 다르게 적용되거나, 잘못된 media URL이 조용히 authored media로 fallback하는 사례가 보고되어 있습니다. 이는 특정 버전의 구현 결함일 수 있지만, Cut에서도 “렌더가 성공했다”보다 “의도한 입력이 최종 artifact에 반영됐다”를 검증해야 한다는 테스트 요구사항을 보여줍니다. [Issue #2064](https://github.com/heygen-com/hyperframes/issues/2064), [Issue #3368](https://github.com/heygen-com/hyperframes/issues/3368), [Issue #2141](https://github.com/heygen-com/hyperframes/issues/2141)

## 현재 구현의 중요한 한계

현재 Cut의 `composition.check`는 첫 단계에서 안전성을 우선해 root의 width, height, fps, duration을 필수로 요구합니다. 반면 HyperFrames의 최신 문서에는 composition duration을 GSAP timeline에서 계산하는 방식과 root `data-duration`을 사용하는 설명이 함께 있어 문서·버전 간 차이가 보입니다. [Core model](https://github.com/heygen-com/hyperframes/blob/main/packages/core/docs/core.md), [Rendering guide](https://github.com/heygen-com/hyperframes/blob/main/docs/guides/rendering.mdx)

따라서 연결된 외부 renderer bridge를 운영하기 전에는 HyperFrames의 특정
버전과 worker 계약을 고정하고, Cut의 manifest와 `composition.check` 계약을
그 버전에 맞춰 정렬해야 합니다. 현재 checker와 bridge를 곧바로 upstream
전체 호환 구현으로 간주하면 안 됩니다.

## 한 줄 우선순위

`template manifest → typed media/variable slots → instance overrides → catalog → proposal/check/preview → deterministic render/cache → canonical timeline 반영`

이 순서라면 HyperFrames의 재사용성과 에이전틱 편집 경험은 가져오면서도, Cut이 이미 갖고 있는 canonical state·provenance·승인 흐름을 보존할 수 있습니다.

## 참고 자료

- [HyperFrames Launches repository](https://github.com/heygen-com/hyperframes-launches)
- [HyperFrames repository](https://github.com/heygen-com/hyperframes)
- [Developer overview](https://github.com/heygen-com/hyperframes/blob/main/docs/developers/overview.mdx)
- [Core composition model](https://github.com/heygen-com/hyperframes/blob/main/packages/core/docs/core.md)
- [Variables and media](https://github.com/heygen-com/hyperframes/blob/main/skills/hyperframes-core/references/variables-and-media.md)
- [Frame adapters](https://github.com/heygen-com/hyperframes/blob/main/docs/concepts/frame-adapters.mdx)
- [Rendering guide](https://github.com/heygen-com/hyperframes/blob/main/docs/guides/rendering.mdx)
- [Engine package](https://github.com/heygen-com/hyperframes/blob/main/docs/packages/engine.mdx)
- [Embedded override mode](https://github.com/heygen-com/hyperframes/blob/main/docs/sdk/guides/embedded-override-mode.mdx)
