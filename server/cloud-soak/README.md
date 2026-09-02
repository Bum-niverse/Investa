# Cloud 24-hour soak jobs

Cloud Run Job에서 PC 전원과 무관하게 실행하는 24시간 검수 전용 이미지다.

- `market`: Upbit 현물, Binance 현물·USDⓈ-M·COIN-M 공개 WebSocket의 수신·stale·재연결을 검증한다.
- `shadow-contract`: 비밀정보와 사용자 DB를 반출하지 않는 격리 SQLite에서 append-only 사건, 트랜잭션, 원장 대사와 `quick_check`를 검증한다.

`shadow-contract`는 Windows 데스크톱 앱, 실제 사용자 SQLite, Windows 자격 증명 관리자 또는 실제 계좌 연결을 검증하지 않는다. 로컬 Tauri 통합 내구 검사의 대체 결과로 표시하면 안 된다.

두 모드 모두 60초마다 구조화된 heartbeat를 Cloud Logging에 남기며, 실제 24시간이 지나야 `actualElapsed24hQualified=true`가 된다. 컨테이너에는 금융·Telegram·Google 비밀정보를 전달하지 않는다.

시장 검사는 `investa.cloud-soak.v2`부터 전송 생존과 시장 이벤트 신선도를 분리한다. Upbit `trade`는 이벤트 기반이므로 20초 체결 공백은 `marketGapEvents`와 최종 `warnings`에 남기되 연결을 닫거나 단독 실패 사유로 쓰지 않는다. 대신 30초마다 공식 `PING` 텍스트를 보내고 `UP` 응답을 `transportHeartbeats`로 분리한다. 실제 close/error 또는 45초간 전송 생존 응답이 없을 때만 재연결하며 `transportTimeouts`는 최종 실패 사유다. Binance 정기 갱신 스트림은 기존 20초 시장 이벤트·전송 제한을 유지한다.

구버전 실행은 배포 당시 이미지와 `investa.cloud-soak.v1` 로그를 그대로 사용한다. 로컬 상태 수집기는 `v1`·`v2` 구조화 완료 로그를 모두 지원하므로 구버전의 검증 결과도 재현 가능하게 판정한다. 실행 중인 작업을 취소하거나 결과를 덮어쓰지 않고, 수정판은 별도 이미지 태그와 실행 ID로 비교한다.

2026-09-02 배포 기록:

- 이미지: `asia-northeast3-docker.pkg.dev/investa-remote-bumniverse/investa/investa-cloud-soak:20260902-1`
- digest: `sha256:6a1081a94c1d256746bae50987189d590e2b2894c64580958b1c1132318ca657`
- 별도 Job: `investa-market-soak-24h-v2`
- 실행: `investa-market-soak-24h-v2-mkr74`
- 초기 60초: Upbit market 183, transport heartbeat 3, reconnect/error/transport timeout 0

위 초기 표본은 배포 경로와 분리 판정만 검증한다. 24시간 실행 결과가 끝나기 전에는 내구 검사 완료로 승격하지 않는다.

로컬 상태 수집은 `pnpm cloud:soak:collect`를 사용한다. Windows에서는 PATH 또는 Google 공식 기본 설치 경로의 `gcloud.cmd`를 찾고, 별도 설치 위치는 절대 경로 `GCLOUD_BIN`으로 지정한다. CLI 부재·로그인 필요·읽기 권한 부족은 각각 수집 불가로 표시하며 Cloud 작업 실패로 오인하지 않는다.

2026-09-03 최종 관측:

- 내부 섀도우 원장 실행 `investa-shadow-contract-soak-24h-x95sr`은 실제 24시간 적격, 사건 1,439건·원장 1,439건·실패 0건·대사 통과로 완료됐다.
- 시장 스트림 실행 `investa-market-soak-24h-v2-mkr74`은 사용자 결정으로 22.68시간에 조기 종료했다. Binance 현물·USDⓈ-M·COIN-M과 Upbit 모두 오류·재연결·전송 timeout 0건이었다.
- 종료 시 누적 메시지는 Binance 현물 976,757건, USDⓈ-M 81,660건, COIN-M 81,660건, Upbit 146,749건이었다. Upbit의 시장 이벤트 공백 163회와 마지막 71.771초 무체결은 전송 장애가 아닌 경고로 보존한다.
- 시장 실행은 24시간을 채우지 않았으므로 `actualElapsed24hQualified=false`이며 24시간 통과로 승격하지 않는다. Cloud Run 사용자 취소는 실제 실패와 구별해 `cancelled`·종합 `warning`으로 표시한다.
