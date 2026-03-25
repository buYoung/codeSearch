# Agent-first 리팩토링 실행 계획

## 요약

이 문서는 설계 승인용 `RFC`가 아니라, implementer가 바로 실행할 수 있는 작업 계획 문서다.

최종 목적은 code agent가 코드를 검색할 때 사람이 읽기 좋은 텍스트가 아니라, 필요한 데이터만 안정적으로 소비할 수 있는 `machine-readable` 표면을 제공하는 것이다.

1차 완료 기준은 `search` 명령을 `JSON CLI` 중심 인터페이스로 전환하는 것이다. `adaptive read`와 `outline` 기반 drill-down은 2차 목표로 두되, 1차부터 내부 데이터 모델은 그 방향으로 맞춘다.

## 현재 기준선

현재 구현 상태는 다음과 같다.

- `search` 단일 명령과 `--mode direct|explore`를 제공한다.
- 출력은 human-readable text 중심이다.
- `JSON` 출력은 아직 없다.
- 지원 언어는 `Rust`, `Go`, `TypeScript`다.
- 현재 저장소에는 실제 `Go`/`TypeScript` fixture가 없다.

실행 기준선은 다음 명령으로 확인했다.

```text
cargo test
```

- 통과
- 단, 테스트 수는 `0`개다.

```text
cargo run -- search ./src CodeSearchService --limit 5
```

- 정상 동작
- `CodeSearchService` exact hit가 반환된다.

```text
cargo run -- search ./src analyze_file --limit 3 --mode explore
```

- 정상 동작
- `analyze_file` exact hit가 반환된다.

```text
cargo run -- search ./src '' --limit 3
```

- validation error 반환
- 현재 오류 메시지는 `query must include at least one searchable token` 이다.

## 고정 결정

이번 리팩토링에서 implementer가 다시 결정하지 않아야 할 항목은 아래와 같다.

- 1차 목표는 `machine-readable output` 이다.
- 기본 소비 표면은 `JSON CLI` 다.
- 기존 text 출력은 제거하지 않고 `--output text`로 임시 유지한다.
- 호환성보다 agent-first 전환을 우선한다.
- 진행 방식은 `Core Split Early` 다.
- 지원 언어 범위는 `Rust`, `Go`, `TypeScript`를 유지한다.
- `Go` method grouping 미완성은 지원 제거 사유가 아니다. 후속 개선 항목으로 남긴다.
- large file 판정 기본값은 token estimate 기반 `10_000` 이다.

## 목표 상태

1차 완료 시점의 공개 표면은 아래와 같다.

```text
code-search search <directoryPath> <query> [--limit <number>] [--mode <direct|explore>] [--output <json|text>]
```

기본 출력은 `json` 이다.

`text` 출력은 비교 검증과 디버깅을 위한 임시 escape hatch로만 유지한다.

top-level `JSON` response는 최소 다음 필드를 포함해야 한다.

- `query`
- `mode`
- `stats`
- `results`

`stats`는 최소 다음 필드를 포함해야 한다.

- `scanned_file_count`
- `matched_target_count`
- `warning_count`

각 `result`는 최소 다음 필드를 포함해야 한다.

- `score`
- `target_kind`
- `symbol_name`
- `file_path`
- `line_start`
- `line_end`
- `semantic_role`
- `sections`
- `raw_target`

`raw_target`은 agent용 기계 필드다. 최소 다음 데이터를 포함해야 한다.

- `signature_text`
- `return_type_hint`
- `parameter_descriptions`
- `incoming_dependencies`
- `outgoing_dependencies`
- `flow_steps`
- `container_name`
- `parent_symbol_name`
- `import_hint`

## 작업 단계

### 1단계: Serializable response type 도입

목적:
현재의 internal model과 `CLI` 출력용 model이 섞여 있으므로, `JSON` 직렬화 가능한 응답 타입을 먼저 분리한다.

작업:
- response 전용 type을 추가한다.
- `search` 결과를 response type으로 변환하는 mapper를 추가한다.
- 기존 `SearchTarget`과 `SearchTraceResult`를 그대로 `CLI` 출력에 직접 넘기지 않도록 경계를 만든다.

완료 조건:
- search 결과를 text renderer 없이도 구조화된 response로 얻을 수 있다.
- 직렬화 대상 field 이름이 이 문서의 목표 상태와 일치한다.

실행 검증:

```text
cargo test
```

### 2단계: Output layer 분리

목적:
`main`에 섞여 있는 parsing, 실행, 결과 분류, text rendering 책임을 분리한다.

작업:
- command parsing 책임과 output rendering 책임을 분리한다.
- text 출력에 필요한 `match label` 계산과 section 렌더링을 별도 output layer로 이동한다.
- 이후 `json` 출력이 같은 core result를 재사용하도록 만든다.

완료 조건:
- `main`은 command parsing, service 호출, output format 선택만 담당한다.
- text renderer는 독립 호출 가능하다.

실행 검증:

```text
cargo test
```

```text
cargo run -- search ./src CodeSearchService --limit 5 --output text
```

### 3단계: Service orchestration 분리

목적:
현재 `service`에 뭉쳐 있는 validation, file discovery, analysis, scoring, trace assembly를 분리해 이후 `read-file` 계열 기능을 붙이기 쉽게 만든다.

작업:
- request validation 분리
- supported file discovery 분리
- per-file analysis aggregation 분리
- scoring/ranking 분리
- trace assembly 분리

완료 조건:
- `search_with_mode`는 orchestration만 담당한다.
- ranking 규칙과 trace 조립 규칙이 독립적으로 테스트 가능하거나 최소 독립 호출 가능하다.

실행 검증:

```text
cargo test
```

```text
cargo run -- search ./src analyze_file --limit 3 --mode explore --output text
```

### 4단계: Outline-ready metadata 추가

목적:
2차 작업인 `adaptive read`를 위해 parser가 outline 노드를 만들 수 있는 최소 관계 정보를 명시적으로 생산하게 만든다.

작업:
- `parent_symbol_name` 또는 이에 대응하는 explicit parent relation을 추가한다.
- 기존 `container_name`, `enclosing_symbol_name`의 역할을 정리한다.
- outline node 생성에 필요한 `children_count` 계산이 가능하도록 metadata를 확보한다.

완료 조건:
- symbol 간 parent-child 관계를 구조적으로 복원할 수 있다.
- `Type`, `Function`, `Method`, `LocalBinding`, `File`이 outline 재조합 가능한 데이터로 남는다.

실행 검증:

```text
cargo test
```

```text
cargo run -- search ./src CodeSearchService --limit 5 --output json
```

### 5단계: `search` 기본 표면을 `JSON`으로 전환

목적:
agent가 바로 소비할 수 있는 1차 목표를 완료한다.

작업:
- `--output json|text`를 도입한다.
- 기본값은 `json` 으로 전환한다.
- 기존 `direct|explore` 정렬 규칙은 유지한다.

완료 조건:
- `search` 기본 실행 결과가 valid `JSON` 이다.
- `text` 출력은 명시적으로 요청했을 때만 나온다.
- direct exact hit 우선 규칙은 유지된다.

실행 검증:

```text
cargo test
```

```text
cargo run -- search ./src CodeSearchService --limit 5
```

```text
cargo run -- search ./src analyze_file --limit 3 --mode explore
```

```text
cargo run -- search ./src '' --limit 3
```

### 6단계: `adaptive read` 준비용 internal contract 추가

목적:
1차 공개 범위를 넘지 않으면서, 다음 단계에서 바로 `read-file`과 `read-range`를 붙일 수 있도록 내부 계약을 고정한다.

작업:
- `ReadFileResponse` internal model 도입
- `response_kind = full_content | outline` union 고정
- `OutlineNode` internal model 도입
- `read-range` 입력/출력 contract 고정

완료 조건:
- large file에서 outline 응답을 반환할 내부 모델이 준비된다.
- outline node에서 range drill-down 가능한 입력 데이터가 준비된다.

실행 검증:

```text
cargo test
```

## 2차 목표 초안

2차 작업에서는 아래 표면을 공개 대상으로 삼는다.

`read-file` response:

- `response_kind`
- `file_path`
- `language`
- `estimated_tokens`
- `content` 또는 `outline`

`OutlineNode` 최소 필드:

- `node_id`
- `parent_node_id`
- `target_kind`
- `symbol_name`
- `file_path`
- `line_start`
- `line_end`
- `children_count`
- `language`

`read-range` 입력:

- `path`
- `line_start`
- `line_end`

`read-range` 출력:

- 잘린 code
- enclosing symbol metadata
- 필요한 경우 parent outline reference

large file 판정 규칙:

- token estimate가 `10_000` 초과면 `full_content` 대신 `outline`
- outline도 token estimate가 `10_000` 초과면 depth를 줄여 `depth=1`까지 축약

small file 예시 검증 대상:

- `src/text.rs`

large file 예시 검증 대상:

- `src/parser.rs`

## 검증 규칙

모든 단계에서 아래 규칙을 따른다.

- 각 단계 종료 후 `cargo test` 실행
- direct exact hit 회귀 여부 확인
- explore mode 결과 생성 회귀 여부 확인
- invalid query validation 회귀 여부 확인
- `JSON` 도입 이후에는 stdout이 valid `JSON` 인지 확인

언어별 검증 규칙은 아래와 같다.

- 현재 repo 안에는 `Go`/`TypeScript` fixture가 없으므로 repo-tracked test file 추가 없이 외부 임시 fixture directory로 smoke 검증한다.
- 리팩토링 중 언어 지원을 임시로 제거하지 않는다.

## 구현 메모

- 이 문서는 작업 순서를 고정하는 실행 문서다.
- 대안 비교나 긴 배경 설명이 필요하면 별도 문서로 분리한다.
- 이번 범위에서는 새 외부 dependency를 최소화한다. `JSON` 직렬화에 필요한 의존성만 허용한다.
