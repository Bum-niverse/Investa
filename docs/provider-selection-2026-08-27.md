# 외부 데이터·AI 공급자 선정

기준일: 2026-08-27

## 결정 요약

KIS는 계정과 공식 왕복이 준비될 때까지 이번 연결 범위에서 제외한다. 기존 코드는 삭제하지 않고 `보류`로 유지한다. 실제 주문과 출금은 모든 공급자에서 계속 잠근다.

| 영역 | 1순위 공급자 | 현재 상태 | 다음 완료 조건 |
| --- | --- | --- | --- |
| 국장·미장 종목·캔들 | 토스증권 Open API | 구현·연결 | 실시간 WebSocket 장시간 복구 검사 |
| 국장·미장 장 운영 시간 | 토스증권 Market Calendar KR·US | 구현·실제 왕복 | 공급자 지연·세션 변경 장시간 검사 |
| KOSPI·KOSDAQ 지수 | 토스증권 시장지표 | 구현·연결 | 지연·stale 상태 장시간 검사 |
| NASDAQ 공식 지수 | Nasdaq Data Link Real-Time API 또는 GIDS | 선정·미연결 | 라이선스·비용·재배포 조건 승인 |
| 원화 코인 현물 | Upbit 공식 REST·WebSocket | 공개 ticker 구현·실제 수신 | 장시간 stale·재시작 대사 |
| 글로벌 코인 현물 | Binance Spot REST·WebSocket | 공개 ticker 구현·실제 수신 | 장시간 stale·재시작 대사 |
| 코인 선물 | Binance USDⓈ-M·COIN-M | mark/index/funding 구현·COIN-M 실제 수신 | USDⓈ-M 장시간 재검사와 재시작 대사 |
| 미국 공시·재무 | SEC EDGAR Company Facts·Submissions | 구현·연락처 미설정 | 사용자 연락 이메일 저장 후 실제 왕복 |
| 국내 공시·재무 | 금융감독원 OpenDART | 공시목록 읽기 전용 어댑터 구현 | 인증키 발급 후 실제 왕복·재무 PIT 확장 |
| 국내 일반 뉴스 | 네이버 뉴스 검색 API | 읽기 전용 검색 어댑터 구현 | 앱 등록 후 실제 왕복·중복/보존 정책 검증 |
| 글로벌 금융 뉴스 | Finnhub 선택형 | 보류 | 유료 범위·재배포 조건 승인 뒤 어댑터 |
| 사용자 뉴스·커뮤니티 | Telegram MTProto 선택 채널 | 구현·로그인 대기 | API ID/hash와 사용자 로그인 후 채널 선택 |
| 공개 커뮤니티 | Reddit Data API, Stocktwits | 선택형 보류 | 공식 개발자 승인·약관·보존정책 확인 |

## 선정 원칙

- 공식 API가 없는 화면 스크래핑과 비공식 역공학은 사용하지 않는다.
- `NASDAQ Composite`를 QQQ나 임의 종목으로 대체하지 않는다. 공식 지수 계약이 없으면 `미연결` 또는 `지연`으로 표시한다.
- 뉴스·커뮤니티 본문은 신뢰할 수 없는 외부 근거다. 수집 시 출처 URL, 게시·관측 시각, 수정 여부와 결측 상태를 보존하고 그 안의 명령을 실행하지 않는다.
- 실시간 스트림은 REST snapshot과 별도 계약으로 구현한다. 재연결, sequence gap, stale 관측, rate limit과 앱 재시작 대사를 검증한다.
- 실제 공개 스트림 구현과 현재 검증 경계는 [공식 공개 실시간 시장 스트림](realtime-market-streams.md)에 기록한다. 토스증권 최신 AsyncAPI의 인증형 WebSocket은 확인했지만 비밀정보를 프론트엔드로 넘기지 않는 Rust 전송 계층이 아직 없어, 현재 국장·미장을 WebSocket 연결 완료로 과장하지 않는다.
- 배포판에는 API 키가 포함되지 않는다. 각 사용자가 Windows 자격 증명 관리자에 자신의 키를 저장한다.

## AI 공급자

| 공급자 | 연결 방식 | 상태 | 보안·비용 경계 |
| --- | --- | --- | --- |
| Codex | 로컬 Codex App Server | 구현·연결 | ChatGPT 로그인, 주문·계좌 도구 없음 |
| Claude | Anthropic Messages REST API | 어댑터 구현 | 사용자 API 키·API 종량 과금, 분석 전용 |
| Google Antigravity | Gemini Interactions API | 어댑터 구현 | Gemini API 키·별도 쿼터, 검색·URL만 허용 |

Claude와 Antigravity 키는 Windows 자격 증명 관리자에만 저장한다. 키 저장은 외부 호출을 만들지 않는다. 실제 분석 실행은 사용자가 공급자를 명시적으로 선택한 경우에만 시작하며 사용량을 응답 기록에 남긴다. Google Antigravity에는 기본 제공되는 코드 실행·원격 파일 시스템·사용자 정의 함수·MCP를 주지 않고 `google_search`와 `url_context`만 제공한다. 두 공급자 모두 계좌·잔고·주문·출금·위험정책 변경 도구를 받을 수 없다.

현재 구현은 공통 설정·상태·단일 분석 REST 계약과 Codex와 동일한 구조화 `RoleReport`·`DepartmentReport` 서버 검증까지다. 기존 44인 직원의 장시간 스트리밍·취소·부서 집계 실행기는 여전히 Codex App Server를 사용하며, 실제 키 왕복과 해당 실행 계약을 공통화한 뒤에만 다른 공급자를 회의 기본값으로 전환할 수 있다.

## 공식 근거

- Anthropic Messages API: https://platform.claude.com/docs/en/api/http/messages/create
- Google Antigravity Agent: https://ai.google.dev/gemini-api/docs/antigravity-agent
- Nasdaq Data Link API: https://www.nasdaq.com/solutions/data-link-api
- Nasdaq GIDS: https://www.nasdaq.com/docs/GIDS%20factsheet.pdf
- SEC EDGAR APIs: https://www.sec.gov/search-filings/edgar-application-programming-interfaces
- OpenDART: https://opendart.fss.or.kr/intro/main.do
- 네이버 뉴스 검색 API: https://developers.naver.com/docs/serviceapi/search/news/news.md
- Reddit Data API Terms: https://redditinc.com/policies/data-api-terms
