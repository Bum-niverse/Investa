# 릴리스 전 보안 검토 — 2026-08-31

## 판정

현재 상태는 **개발 검증 후보**다. 소스 공개·협업 정리는 가능하지만 공식 설치 파일 배포, 실주문, Apple 배포와 macOS 지원을 선언할 단계는 아니다. 실제 주문 전송은 구현하지 않았고 모든 주문 후보는 `SHADOW ONLY` 안에 남는다.

## 감사 범위와 위협 모델

- 자산: 금융 API 자격정보, OAuth 소유자 식별자, Telegram·Cloud secret, 계좌 식별정보, SQLite 연구·모의원장, Codex/GitHub 세션
- 경계: React↔Tauri IPC, 운영체제 Keychain, SQLite, Codex/GitHub CLI 하위 프로세스, 금융 공급자 HTTPS/WSS, Telegram webhook, Cloud relay
- 오용 경로: 비밀정보 Git 추적·로그 노출, 변조된 IPC 입력, 다른 계정의 통합 작업공간 접근, webhook 재전송, 주문 권한 오인, 장시간 프로세스·원장 불일치

## 이번 정리

- Windows 자격 증명 관리자와 macOS Keychain을 기존 `keyring` 인터페이스로 유지했다.
- Unix 앱 데이터 디렉터리를 현재 사용자 전용 `0700`으로 제한했다.
- macOS 내부 내구 검사에서 현재 프로세스 resident memory를 Mach API로 직접 읽도록 했다.
- GitHub Windows 검증 workflow의 잘못된 pnpm action을 공식 `pnpm/action-setup`과 `actions/setup-node` 조합으로 교정했다.
- 비용이 발생할 수 있는 private macOS runner는 수동 실행 workflow로 분리했다.
- 회의 중단·복구·재개·완료를 100회 반복하는 결정론적 회귀 테스트를 추가했다.
- 보안 정책에 지원 버전, 코드 서명·notarization 미완료 상태와 비공개 취약점 제보 경계를 명시했다.

## 서명·운영 적용 상태

| 항목 | 상태 |
| --- | --- |
| Git 커밋 서명 | 로컬 서명 키 미설정 — 서명됐다고 주장하지 않음 |
| Windows 설치 파일 코드 서명 | 미수행 |
| Apple Developer ID 서명 | 미수행 |
| Apple notarization | 미수행 |
| Google Cloud 배포 | 이번 범위에서 보류 |
| 실주문 | 코드·UI 모두 잠금 유지 |

이 문서는 암호학적 서명이나 제3자 보안 인증서가 아니다. 검증 명령과 커밋으로 재현 가능한 유지관리자 자체 점검 기록이다.

## 완료 조건

- frontend test/build, Rust fmt/test, relay test가 통과한다.
- `cargo audit`, 저장소 보안 감사와 Git diff 비밀 검사가 통과한다.
- 공개 시장 스트림과 내부 섀도우 실제 24시간 결과는 시뮬레이션과 분리돼 보존된다.
- GitHub에 SQLite, `.env`, 키·토큰·계좌번호, 빌드 산출물과 로컬 감사 원문이 포함되지 않는다.

## 미검증·잔여 위험

- 공개 시장 스트림 24시간 시험은 2026-08-31 03:46 KST 시작 상태이며 실제 종료 결과 전에는 통과로 표시하지 않는다.
- 내부 섀도우 실제 24시간 시험은 앱에서 시작해야 하며, 이번 작업은 사용자 화면 조작 금지 조건 때문에 자동 시작하지 않았다.
- macOS 코드는 실제 Mac runner와 실기기에서 아직 검증되지 않았다.
- Private Vulnerability Reporting 활성 상태는 확인되지 않았다.
- 코드 서명, notarization과 공식 설치 파일 공급망은 배포 단계에서 별도 승인·설정이 필요하다.
