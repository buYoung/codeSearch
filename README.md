# code-search

[![Rust](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`tree-sitter`와 `tantivy`를 활용한 구조 인식 코드 검색 CLI입니다. 디렉터리를 스캔하여 함수, 메서드, 타입, 로컬 바인딩 단위의 검색 타겟을 추출하고 BM25 기반 랭킹으로 결과를 반환합니다. AI agent가 바로 소비할 수 있는 JSON 출력을 기본으로 제공합니다.

## Table of Contents

- [Features](#features)
- [Supported Languages](#supported-languages)
- [Architecture](#architecture)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Usage](#usage)
- [Output Formats](#output-formats)
- [Response Schema](#response-schema)
- [Documentation](#documentation)
- [License](#license)

## Features

- **구조 인식 파싱** — `tree-sitter`로 function, method, type, local binding 단위 타겟 추출
- **BM25 랭킹** — `tantivy` 인메모리 인덱스에서 symbol, signature, context 필드별 boost 적용
- **검색 모드** — `direct` (정확 매칭 우선) / `explore` (관련 컨텍스트 포함 탐색)
- **Agent 친화 JSON** — `schema_version`, `target_id`, 구조화된 `sections.location`, `raw_target` 메타데이터 포함
- **병렬 처리** — `rayon`으로 파일별 파싱/타겟 추출 병렬화
- **gitignore 존중** — `.gitignore` 및 기본 ignore 규칙 적용

## Supported Languages

| Language   | Extensions       |
|------------|------------------|
| Rust       | `.rs`            |
| Go         | `.go`            |
| TypeScript | `.ts`, `.tsx`    |

## Architecture

```
src/
├── main.rs              # CLI 진입점 (clap 기반)
├── lib.rs               # 모듈 선언
├── model.rs             # 핵심 도메인 모델 (SearchTarget, SearchHit, SearchResults 등)
├── text.rs              # 텍스트 유틸리티
├── parser/              # tree-sitter 기반 언어별 파서
│   ├── mod.rs
│   ├── rust.rs
│   ├── go.rs
│   └── typescript.rs
├── search/              # 검색 엔진 코어
│   ├── mod.rs           # CodeSearchService
│   ├── discovery.rs     # 파일 디스커버리
│   ├── ranking.rs       # tantivy BM25 랭킹
│   ├── trace.rs         # 결과 trace/section 구성
│   └── validation.rs    # 입력 검증
└── output/              # 출력 렌더링
    ├── mod.rs           # OutputFormat, render_search_output
    ├── response.rs      # JSON 응답 구조체
    ├── json.rs          # JSON 렌더러
    └── text.rs          # Human-readable 텍스트 렌더러
```

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) toolchain (edition 2024)
- `cargo`

## Installation

```bash
git clone https://github.com/<your-username>/code-search.git
cd code-search
cargo build --release
```

빌드된 바이너리는 `./target/release/code-search`에 생성됩니다.

## Usage

```
code-search search <DIRECTORY_PATH> <QUERY> [OPTIONS]
```

### Options

| Option     | Default   | Description                          |
|------------|-----------|--------------------------------------|
| `--limit`  | `10`      | 반환할 최대 결과 수                  |
| `--mode`   | `direct`  | 검색 모드 (`direct` \| `explore`)    |
| `--output` | `json`    | 출력 형식 (`json` \| `text`)         |

### Examples

```bash
# 현재 디렉터리에서 "search" 키워드 검색
cargo run -- search . search --limit 3

# src/ 하위에서 특정 심볼 검색
cargo run -- search ./src CodeSearchService --limit 5

# explore 모드로 관련 컨텍스트 탐색
cargo run -- search ./src analyze_file --limit 5 --mode explore

# human-readable 텍스트 출력
cargo run -- search ./src CodeSearchService --limit 5 --output text

# 다중 단어 쿼리
cargo run -- search ~/workspace/my-repo "http client retry" --limit 10
```

> **Tip:** 공백이 포함된 쿼리는 따옴표로 감싸세요.

## Output Formats

### JSON (default)

```json
{
  "schema_version": 1,
  "query": "CodeSearchService",
  "mode": "direct",
  "stats": {
    "scanned_file_count": 17,
    "matched_target_count": 172,
    "warning_count": 0
  },
  "results": [
    {
      "score": 15.078,
      "target_id": "search/mod.rs#L11-L11:type:CodeSearchService",
      "target_kind": "type",
      "symbol_name": "CodeSearchService",
      "file_path": "search/mod.rs",
      "language": "rust",
      "line_start": 11,
      "line_end": 11,
      "sections": [ ... ],
      "raw_target": { ... }
    }
  ]
}
```

### Text (`--output text`)

```
Scanned files: 17
Matched targets: 172
Warnings: 0

질문: 'CodeSearchService'
Mode: direct

━━━ Direct matches ━━━

결과 1  type  CodeSearchService  search/mod.rs:11  [exact]  score=15.079

━━━ 선언 ━━━
  pub struct CodeSearchService
    → 위치: search/mod.rs:11

━━━ 사용법 ━━━
  use crate::search::CodeSearchService;
```

### Error Response

런타임 에러 발생 시 JSON 에러 응답과 non-zero exit code를 반환합니다.

```json
{
  "error": {
    "kind": "invalid_request",
    "message": "query must include at least one searchable token"
  }
}
```

## Response Schema

| Field | Description |
|-------|-------------|
| `schema_version` | 응답 스키마 버전 |
| `query` | 실행된 검색 쿼리 |
| `mode` | 검색 모드 (`direct` \| `explore`) |
| `stats.scanned_file_count` | 스캔된 지원 파일 수 |
| `stats.matched_target_count` | 매칭된 타겟 수 |
| `stats.warning_count` | 파싱 실패 등 fallback 처리 횟수 |
| `results[].target_id` | `file + line range + kind + symbol` 기반 식별자 |
| `results[].target_kind` | `function` \| `method` \| `type` \| `local_binding` |
| `results[].sections[].entries[].location` | `file_path`, `line_start`, `line_end` 구조화 위치 |
| `results[].raw_target` | signature, dependency, flow, parent symbol 등 메타데이터 |

## License

이 프로젝트는 [MIT License](LICENSE) 하에 배포됩니다.
