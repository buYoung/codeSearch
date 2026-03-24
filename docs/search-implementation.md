# Search 구현 메모

## 한 줄 요약

- 현재 구현은 `search` 단일 명령만 제공하는 Search-First `CLI`다
- 입력 디렉터리를 즉석 스캔하고, `tree-sitter`로 target을 만든 뒤 in-memory `tantivy` `BM25`로 검색한다
- 결과는 file aggregation이 아니라 hit 중심 trace 형식으로 출력한다

## 현재 구현 상태

- `search` 단일 명령만 구현되어 있다
- 입력 디렉터리는 실행 시점에 즉석 스캔한다
- 파일 수집은 `.gitignore`와 기본 ignore 규칙을 존중한다
- 지원 확장자는 `.rs`, `.go`, `.ts`, `.tsx`다
- 파일별 parsing/chunk 추출은 `rayon`으로 병렬 처리한다
- `tree-sitter`로 function, method, type, local binding target을 추출한다
- ranking은 in-memory `tantivy`의 `BM25`를 사용한다
- 로컬 변수 hit는 `선언 / 데이터 흐름 / 의존성`, 함수 hit는 `구현 / 상위 호출지점 / 의존성`으로 출력한다
- query가 비어 있거나 `limit`가 `0`이면 오류를 반환한다

## 실행 명령

```text
cargo run -- search <directoryPath> <query> --limit 10
```

```text
cargo build
./target/debug/code-search search <directoryPath> <query> --limit 10
```

## 예시 명령

```text
cargo run -- search . search --limit 3
```

```text
cargo run -- search ./src CodeSearchService --limit 5
```

```text
cargo run -- search ~/workspace/my-repo "http client retry" --limit 10
```

- 공백이 포함된 query는 따옴표로 감싸서 실행한다

## 출력 형식

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

- `tree-sitter`로 function, method, type, local binding target을 추출한다
- 파일별 분석은 `rayon` `par_iter()`로 병렬 실행한다
- 각 target에서 선언, dependency, same-scope flow 정보를 검색용 텍스트로 만든다
- in-memory `tantivy` 인덱스에 target을 넣고 `BM25`로 점수화한다
- score 상위 target을 trace section으로 조립해 출력한다

## 관련 문서

- 설계 배경: [code-search-poc.md](/Users/buyonglee/RustroverProjects/code-search/docs/code-search-poc.md)
- 사용 안내: [README.md](/Users/buyonglee/RustroverProjects/code-search/README.md)
