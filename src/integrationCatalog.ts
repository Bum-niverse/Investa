export type IntegrationKind = "broker" | "exchange" | "ai" | "disclosure" | "news" | "community" | "index";
export type IntegrationCapability = "market-data" | "account-read" | "paper-order" | "live-order" | "analysis" | "disclosures" | "news" | "community";
export type IntegrationSupport = "built-in" | "partial" | "adapter-required";

export type IntegrationDefinition = {
  id: string;
  name: string;
  kind: IntegrationKind;
  markets: Array<"kr" | "us" | "coin" | "futures">;
  capabilities: IntegrationCapability[];
  support: IntegrationSupport;
  summary: string;
};

export const INTEGRATION_CATALOG: IntegrationDefinition[] = [
  { id: "toss", name: "토스증권", kind: "broker", markets: ["kr", "us"], capabilities: ["market-data", "account-read"], support: "built-in", summary: "국장·미장 시세, 계좌·보유자산 조회" },
  { id: "upbit", name: "Upbit", kind: "exchange", markets: ["coin"], capabilities: ["market-data", "account-read"], support: "built-in", summary: "코인 공개 시세·캔들, 개인계좌 읽기 전용 조회" },
  { id: "binance", name: "Binance", kind: "exchange", markets: ["coin", "futures"], capabilities: ["market-data", "account-read"], support: "built-in", summary: "현물·USDⓈ-M·COIN-M 공개 시세와 계좌·포지션 읽기 전용 조회" },
  { id: "sec", name: "SEC EDGAR", kind: "disclosure", markets: ["us"], capabilities: ["disclosures"], support: "built-in", summary: "미국 공식 공시·Company Facts 재무" },
  { id: "telegram", name: "Telegram", kind: "community", markets: ["kr", "us", "coin", "futures"], capabilities: ["news", "community"], support: "built-in", summary: "사용자가 선택한 방송 채널 읽기 전용 수집" },
  { id: "opendart", name: "OpenDART", kind: "disclosure", markets: ["kr"], capabilities: ["disclosures"], support: "adapter-required", summary: "국내 공식 공시·재무 1순위 어댑터" },
  { id: "naver-news", name: "네이버 뉴스 검색", kind: "news", markets: ["kr"], capabilities: ["news"], support: "adapter-required", summary: "국내 일반 뉴스 공식 검색 API 1순위" },
  { id: "nasdaq-data-link", name: "Nasdaq Data Link / GIDS", kind: "index", markets: ["us"], capabilities: ["market-data"], support: "adapter-required", summary: "NASDAQ 공식 지수 · 라이선스 확정 후 연결" },
  { id: "reddit", name: "Reddit Data API", kind: "community", markets: ["us", "coin"], capabilities: ["community"], support: "adapter-required", summary: "공식 개발자 승인·약관 확인 후 선택 연결" },
  { id: "stocktwits", name: "Stocktwits", kind: "community", markets: ["us"], capabilities: ["community"], support: "adapter-required", summary: "공식 개발자 접근 승인 후 선택 연결" },
  { id: "codex", name: "Codex", kind: "ai", markets: ["kr", "us", "coin", "futures"], capabilities: ["analysis"], support: "built-in", summary: "사용자의 Codex CLI 로그인으로 읽기 전용 분석" },
  { id: "claude", name: "Claude API", kind: "ai", markets: ["kr", "us", "coin", "futures"], capabilities: ["analysis"], support: "partial", summary: "분석 전용 REST 어댑터 · 사용자 API 키 필요" },
  { id: "antigravity", name: "Google Antigravity", kind: "ai", markets: ["kr", "us", "coin", "futures"], capabilities: ["analysis"], support: "partial", summary: "검색·URL 근거 분석 전용 · 사용자 Gemini API 키 필요" },
  { id: "custom", name: "기타 증권사·거래소·AI", kind: "broker", markets: ["kr", "us", "coin", "futures"], capabilities: [], support: "adapter-required", summary: "공급자의 공식 API별 어댑터 구현 필요" },
];

export const supportLabel: Record<IntegrationSupport, string> = {
  "built-in": "지원",
  partial: "일부 지원",
  "adapter-required": "어댑터 필요",
};
