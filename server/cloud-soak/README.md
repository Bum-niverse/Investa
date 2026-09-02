# Cloud 24-hour soak jobs

Cloud Run Job에서 PC 전원과 무관하게 실행하는 24시간 검수 전용 이미지다.

- `market`: Upbit 현물, Binance 현물·USDⓈ-M·COIN-M 공개 WebSocket의 수신·stale·재연결을 검증한다.
- `shadow-contract`: 비밀정보와 사용자 DB를 반출하지 않는 격리 SQLite에서 append-only 사건, 트랜잭션, 원장 대사와 `quick_check`를 검증한다.

`shadow-contract`는 Windows 데스크톱 앱, 실제 사용자 SQLite, Windows 자격 증명 관리자 또는 실제 계좌 연결을 검증하지 않는다. 로컬 Tauri 통합 내구 검사의 대체 결과로 표시하면 안 된다.

두 모드 모두 60초마다 구조화된 heartbeat를 Cloud Logging에 남기며, 실제 24시간이 지나야 `actualElapsed24hQualified=true`가 된다. 컨테이너에는 금융·Telegram·Google 비밀정보를 전달하지 않는다.

시장 검사는 `investa.cloud-soak.v2`부터 전송 생존과 시장 이벤트 신선도를 분리한다. Upbit `trade`는 이벤트 기반이므로 20초 체결 공백은 `marketGapEvents`와 최종 `warnings`에 남기되 연결을 닫거나 단독 실패 사유로 쓰지 않는다. 대신 30초마다 공식 `PING` 텍스트를 보내고 `UP` 응답을 `transportHeartbeats`로 분리한다. 실제 close/error 또는 45초간 전송 생존 응답이 없을 때만 재연결하며 `transportTimeouts`는 최종 실패 사유다. Binance 정기 갱신 스트림은 기존 20초 시장 이벤트·전송 제한을 유지한다.

구버전 실행은 배포 당시 이미지와 `investa.cloud-soak.v1` 로그를 그대로 사용한다. 실행 중인 작업을 취소하거나 결과를 덮어쓰지 않고, 수정판은 별도 이미지 태그와 실행 ID로 비교한다.

2026-09-02 배포 기록:

- 이미지: `asia-northeast3-docker.pkg.dev/investa-remote-bumniverse/investa/investa-cloud-soak:20260902-1`
- digest: `sha256:6a1081a94c1d256746bae50987189d590e2b2894c64580958b1c1132318ca657`
- 별도 Job: `investa-market-soak-24h-v2`
- 실행: `investa-market-soak-24h-v2-mkr74`
- 초기 60초: Upbit market 183, transport heartbeat 3, reconnect/error/transport timeout 0

위 초기 표본은 배포 경로와 분리 판정만 검증한다. 24시간 실행 결과가 끝나기 전에는 내구 검사 완료로 승격하지 않는다.
