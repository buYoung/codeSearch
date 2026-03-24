# Rust 코드 검색 Search-First PoC

## 한 줄 요약

- `Rust`로 만드는 개인 프로젝트용 코드 검색 `CLI` PoC
- 외부 표면은 `search` 한 명령만 제공한다
- `tree-sitter`로 `TypeScript`, `Go`, `Rust` 파일에서 symbol/function/local binding target을 추출한다
- 각 `search` 실행은 디렉터리를 즉석에서 스캔하고 in-memory `tantivy` 인덱스로 `BM25` 검색을 수행한다
- 결과는 hit 중심 trace로 보여주고, 선언/구현/데이터 흐름/의존성으로 매칭 이유를 설명한다

## PoC 목적

- 이번 PoC는 persistent `index`를 먼저 만드는 작업이 아니다
- 목표는 search 품질과 결과 설명력을 빠르게 검증하는 것이다
- 사용자는 `search`만 이해하면 되고, `tree-sitter`와 `tantivy` 세부 구현은 내부에 숨긴다
- `grep`보다 구조를 더 이해한 결과를 보여주되 구현 복잡도는 낮게 유지한다

## 문제 정의

- 단순 문자열 검색은 function, method, type 같은 코드 경계를 모른다
- 같은 키워드라도 symbol name, signature, comment, identifier 위치에 따라 의미가 달라진다
- 파일 전체를 그대로 보여주면 왜 이 파일이 상위에 왔는지 설명하기 어렵다
- 반대로 PoC 단계에서 semantic search나 reranking까지 넣으면 검증 포인트가 흐려진다
- 그래서 이번 단계는 AST 기반 청크 분리와 `tantivy`의 `BM25` ranking만으로 설명 가능한 search를 목표로 한다

## 기술 선택

|기술|역할|채택 이유|
|---|---|---|
|`Rust`|기본 구현 언어|`CLI` 구현에 적합하고, 성능과 메모리 안정성을 챙기기 좋다|
|`tree-sitter`|코드 파싱과 청크 분리|`TypeScript`, `Go`, `Rust`를 공통된 방식으로 파싱하고 symbol/function 경계를 뽑아내기 좋다|
|`rayon`|파일 단위 병렬 분석|로컬 디렉터리 스캔 후 parsing/chunk 추출을 단순하게 병렬화하기 좋다|
|`tantivy`|in-memory `BM25` 검색 엔진|직접 scorer를 구현하지 않고도 chunk 단위 lexical ranking을 빠르게 검증할 수 있다|

## 범위

- 입력 대상: 로컬 디렉터리
- 지원 언어: `TypeScript`, `Go`, `Rust`
- 실행 형태: `CLI only`
- 검색 결과 단위: hit 중심 trace
- 보조 정보: declaration/implementation/data flow/dependency
- 최소 명령: `search`
- 파싱 실패 처리: 일부 파일이 실패해도 전체 search는 중단하지 않는다

## 예상 CLI 표면

```text
code-search search <directoryPath> <query> --limit 10 --mode direct
```

- `query`는 여러 단어를 받을 수 있다
- `limit`는 파일 결과 개수에만 적용한다
- `mode`는 `direct`와 `explore` 중 하나이며 기본값은 `direct`다

## 아키텍처 개요

### Public Entry

- `CodeSearchService`
  - 외부에 노출되는 유일한 진입점
  - `search`를 제공한다

### Internal Components

- `LanguageParser`
  - 언어 판별과 `tree-sitter` 파싱 담당
- `ChunkExtractor`
  - symbol/function/local binding target 추출 담당
- `TantivySearchIndex`
  - target을 in-memory `tantivy` 문서로 만들고 `BM25` 검색을 수행한다
- `TraceAssembler`
  - 검색된 target을 hit 중심 trace 결과로 조립한다

## 검색 흐름

1. 사용자가 `directoryPath`, `query`, `limit`를 입력한다
2. 지원 확장자 기준으로 대상 파일을 수집한다
3. `rayon`으로 파일 단위 parsing 작업을 병렬 실행한다
4. 각 파일을 `tree-sitter`로 파싱한다
5. function, method, type, local binding target을 추출한다
6. 각 target에서 선언, comment, identifier token, dependency, same-scope flow 정보를 검색용 텍스트로 만든다
7. in-memory `tantivy` 인덱스에 target 문서를 추가한다
8. `tantivy`의 multi-field `BM25` ranking으로 target을 스코어링한다
9. `direct` 모드에서는 exact symbol hit를 먼저, `explore` 모드에서는 score 중심으로 결과를 정렬한다
10. score 상위 target을 hit 중심 trace 결과로 조립한다
11. local binding hit는 `선언 / 데이터 흐름 / 의존성`, callable hit는 `구현 / 상위 호출지점 / 의존성`으로 보여준다

## 개념 인터페이스

```text
search(directoryPath, query, limit) -> SearchResults
```

- `SearchRequest`
  - `directoryPath`
  - `query`
  - `limit`
- `SupportedLanguage`
  - `TypeScript | Go | Rust`
- `SearchTarget`
  - `file path`
  - `language`
  - `symbol name`
  - `target kind`
  - `line range`
  - `searchable text`
- `trace metadata`
- `SearchTraceResult`
  - `score`
  - `sections`

## 구현 기본값

- `query`는 코드 키워드와 symbol 검색을 우선 대상으로 본다
- `tree-sitter`는 구조 검색 DSL이 아니라 청크 분리와 토큰 정리 용도로 사용한다
- parse 또는 target 추출이 실패한 파일은 file-level fallback target으로 검색 대상에 포함한다
- 출력은 사람이 읽는 텍스트 형식으로 고정한다
- `JSON` 출력, persistent cache, `index` 명령은 후속 단계로 미룬다

## 검증 시나리오

- `TypeScript`, `Go`, `Rust`가 섞인 디렉터리에서 `search` 한 번으로 결과가 나와야 한다
- function name, type name, comment keyword 질의 시 관련 파일이 상위에 나와야 한다
- 결과는 hit 중심 trace로 정렬되되, 관련 선언/구현/흐름이 함께 보여야 한다
- 일부 파일 파싱 실패가 있어도 전체 명령이 실패하지 않아야 한다
- 사용자는 결과만 보고도 어떤 symbol이 매칭되었는지 이해할 수 있어야 한다

## 제외 범위

- persistent `index`
- vector search
- semantic search
- hybrid reranking
- remote repository 연동
- Web UI
- 구조 검색 DSL
- IDE 또는 editor plugin 연동
