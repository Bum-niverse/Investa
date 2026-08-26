# Investa 작업 지침

이 저장소에서 작업하는 Codex는 작업 시작 전에 다음 문서를 순서대로 읽는다.

1. `HANDOFF.md`
2. `README.md`
3. `SECURITY.md`
4. 요청과 관련된 `docs/` 문서

## 제품 경계

- Investa는 로컬 우선 Windows 데스크톱 투자 연구·백테스트·내부 모의투자 프로그램이다.
- 실제 주문과 출금은 항상 잠근다. `SHADOW ONLY` 경계를 우회하거나 실주문 전송 함수를 추가하지 않는다.
- LLM은 분석과 제안만 수행한다. 주문, 위험 정책 변경, 계좌 권한과 자격정보를 LLM에 제공하지 않는다.
- 수익률·승률·상승 확률을 보장하거나 데이터가 없을 때 값을 꾸며내지 않는다.
- 백테스트, 모의원장, 실제 계좌 조회와 실전 주문을 명확히 구분한다.

## 보안

- 토큰, API Key·Secret, 전체 계좌번호, Telegram 사용자 ID, webhook secret, Google Cloud secret 원문을 코드·문서·테스트·로그·Git에 기록하지 않는다.
- 로컬 비밀정보는 Windows 자격 증명 관리자, Cloud 비밀정보는 Google Secret Manager에서만 관리한다.
- `.env`, SQLite DB, ProjectStudio DB, 서비스 계정 키와 빌드 산출물을 커밋하지 않는다.
- 외부 입력, Telegram webhook, Cloud relay와 금융 API 변경 시 `SECURITY.md`와 관련 테스트를 함께 확인한다.

## 구현 원칙

- 기존 Tauri 2 + React 19 + TypeScript + Vite + Rust + SQLite 구조를 유지한다.
- 새 라이브러리나 유료 API는 도입 전에 목적·라이선스·비용·대안을 사용자에게 설명하고 승인을 받는다.
- 기존 타입, 서비스, 테스트와 디자인 토큰을 우선 재사용하고 관련 없는 대규모 리팩터링을 피한다.
- 투자 데이터는 point-in-time 정합성, 출처, 관측 시각, 결측 상태와 재현성을 보존한다.
- ProjectStudio 기획을 수정할 때는 로컬 DB를 Git에 복사하지 말고 `scripts/sync_projectstudio_*.py` 방식으로 멱등 동기화한다.

## 완료 전 검증

```powershell
pnpm test:frontend
pnpm build
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
Push-Location server/relay; node --test; Pop-Location
```

변경 파일과 실제 diff를 검토하고, 비밀정보·로컬 DB·생성 산출물이 포함되지 않았는지 확인한다. 실행하지 못한 검사는 완료로 보고하지 않는다.
