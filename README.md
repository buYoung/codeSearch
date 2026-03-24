# code-search

`tree-sitter`와 `tantivy`를 사용해 `Rust`, `Go`, `TypeScript` 코드를 검색하는 Search-First `CLI` PoC입니다. 실행 시점에 디렉터리를 스캔하고, 함수/메서드/로컬 binding 단위 target을 만든 뒤 `BM25`로 hit 중심 trace 결과를 반환합니다.

자세한 설계 배경은 [docs/code-search-poc.md](/Users/buyonglee/RustroverProjects/code-search/docs/code-search-poc.md), 현재 구현 메모는 [docs/search-implementation.md](/Users/buyonglee/RustroverProjects/code-search/docs/search-implementation.md)를 참고하면 됩니다.

## 현재 상태

- `search` 명령만 지원합니다
- 지원 확장자: `.rs`, `.go`, `.ts`, `.tsx`
- 디렉터리 스캔 시 `.gitignore`와 기본 ignore 규칙을 존중합니다
- 결과는 hit 중심 trace 형식으로 출력되고, 선언/데이터 흐름/의존성 또는 구현/상위 호출지점을 함께 보여줍니다
- persistent `index`, `JSON` 출력, semantic search는 아직 구현하지 않았습니다

## 요구 사항

- `Rust` toolchain
- `cargo`

## 실행 명령어

개발 실행:

```text
cargo run -- search <directoryPath> <query> --limit 10
```

빌드 후 실행:

```text
cargo build
./target/debug/code-search search <directoryPath> <query> --limit 10
```

도움말에 해당하는 usage:

```text
code-search search <directoryPath> <query> [--limit <number>]
```

- 공백이 들어가는 query는 `"http client retry"`처럼 따옴표로 감싸는 편이 안전합니다

## 예시 명령어

현재 저장소에서 `search` 키워드 찾기:

```text
cargo run -- search . search --limit 3
```

현재 저장소에서 `CodeSearchService` 심볼 찾기:

```text
cargo run -- search . CodeSearchService --limit 5
```

`src` 디렉터리만 대상으로 `parse` 관련 코드 찾기:

```text
cargo run -- search ./src parse --limit 5
```

여러 단어 query로 검색:

```text
cargo run -- search ~/workspace/my-repo "http client retry" --limit 10
```

## 출력 예시

```text
Scanned files: 1
Matched targets: 4
Warnings: 0

결과 1  score=8.550
질문: 'clinicalReviewQuery'

━━━ 선언 ━━━
  const clinicalReviewQuery = this.rpm99091Repository.findRPM99091ClinicalReview(query);
    → 위치: getClinicalReview() @ rpm99091.service.ts:5

━━━ 데이터 흐름 ━━━
  clinicalReviewQuery
  ↓ combineLatest([clinicalReviewQuery, patientInfoQuery])
  ↓ map(([clinicalReviewData, patientInfo]) => { ... })
```

- `Scanned files`: 검색 대상으로 본 지원 파일 수
- `Matched targets`: 검색어와 매칭된 target 수
- `Warnings`: parse 실패 등으로 fallback 처리한 횟수

## 구현 요약

- `tree-sitter`로 function, method, type, local binding target을 추출합니다
- 파일별 parsing/target 추출은 `rayon`으로 병렬 처리합니다
- 각 target에서 선언, direct dependency, same-scope data flow, callable dependency를 검색용 텍스트로 만듭니다
- in-memory `tantivy` 인덱스에 target을 넣고 `BM25`로 점수화한 뒤 trace 결과로 렌더링합니다
