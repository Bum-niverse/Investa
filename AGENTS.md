# Investa 작업 지침

이 저장소에서 작업하는 Codex는 작업 시작 전에 다음 문서를 순서대로 읽는다.

1. `HANDOFF.md`
2. `README.md`
3. `SECURITY.md`
4. 요청과 관련된 `docs/` 문서

## 모든 작업의 필수 사전 게이트

제품 코드, 데이터, 문서, UI, 기획을 변경하기 전에 예외 없이 다음 순서를 지킨다.

1. 현재 저장소·테스트·ProjectStudio 기획과 사용자 변경을 먼저 확인한다.
2. `SECURITY.md`를 기준으로 변경 대상의 자산, 신뢰 경계, 외부 입력, 인증·인가, 비밀정보, 로그, 파일·DB, 네트워크, 비용·남용, 롤백 가능성을 사전 검토한다.
3. 치명적 노출, 권한 우회, 비밀정보 유출 또는 비가역 데이터 손상 가능성이 있으면 기능 구현보다 차단과 사용자 보고를 우선한다.
4. GitHub, Kaggle, Google 검색을 각각 확인해 관련 공식 문서·원 논문·공개 구현·데이터셋 사례를 조사한다. 관련성이 없는 출처는 억지로 적용하지 않고 `적용 가능한 결과 없음`으로 기록한다.
5. Google 검색 결과와 블로그는 발견 수단일 뿐 근거 자체로 사용하지 않는다. 가능한 경우 공급자 공식 문서, 원 논문, upstream 저장소와 보안 권고로 다시 검증한다.
6. GitHub 후보는 upstream 여부, 최근 release·commit, 유지보수 상태, Security 문서·취약점, 의존성, 라이선스를 확인한다. 코드를 내려받거나 실행하기 전에 출처와 고정 revision을 검증한다.
7. Kaggle 후보는 데이터·모델·Notebook의 소유자, 버전, 라이선스, 데이터 카드·컬럼 설명, 수집 시점, 누수·생존편향과 재배포 가능성을 확인한다. Kaggle 점수와 Notebook 결과를 제품 성능 근거로 간주하지 않는다.
8. 조사 결과에서 채택·부분 채택·보류·기각을 구분하고 이유, 적용 범위, 라이선스, 보안 영향과 검증 방법을 `docs/development-reference-policy.md` 또는 관련 작업 문서에 남긴 뒤 구현한다.
9. 구현 후 정상·경계·실패·권한 부족·변조 입력·재전송·데이터 누수와 비밀정보 검사를 수행하고 실제 diff를 검토한다.

작은 문구·스타일 변경도 보안 영향 여부와 세 출처의 관련성 확인은 생략하지 않는다. 다만 조사 깊이는 위험과 변경 규모에 비례하며, 무관한 레퍼런스를 기능에 끼워 넣지 않는다.

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
