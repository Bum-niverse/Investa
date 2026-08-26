export type AnalysisMarket = "kr" | "us" | "coin" | "securities_futures" | "crypto_futures" | "mixed";
export type ForecastAssetClass = "korea_stock" | "united_states_stock" | "equity_future" | "index_future" | "crypto_spot" | "crypto_perpetual";

const CRYPTO_PATTERN = /(?:코인|암호화폐|가상자산|비트코인|이더리움|btc|eth|usdt|crypto)/i;
const FUTURES_PATTERN = /(?:선물|무기한|perpetual|futures?)/i;
const SECURITIES_FUTURES_PATTERN = /(?:주식\s*선물|지수\s*선물|선물\s*옵션|코스피\s*200|kospi\s*200|국내\s*선물|해외\s*선물|equity\s*future|index\s*future)/i;
const UNITED_STATES_PATTERN = /(?:미장|미국\s*주식|나스닥|nasdaq|nyse|s&p|애플|마이크로소프트|엔비디아|apple|aapl|msft|nvda)/i;
const KOREA_PATTERN = /(?:국장|국내\s*주식|코스피|코스닥|kospi|kosdaq|삼성전자|하이닉스|한화)/i;

export function inferAnalysisMarket(text: string | null | undefined): AnalysisMarket {
  const value = text?.trim() ?? "";
  if (!value) return "mixed";
  if (CRYPTO_PATTERN.test(value) && FUTURES_PATTERN.test(value)) return "crypto_futures";
  if (SECURITIES_FUTURES_PATTERN.test(value)) return "securities_futures";
  if (CRYPTO_PATTERN.test(value)) return "coin";

  const isUnitedStates = UNITED_STATES_PATTERN.test(value);
  const isKorea = KOREA_PATTERN.test(value);
  if (isUnitedStates && isKorea) return "mixed";
  if (isUnitedStates) return "us";
  if (isKorea) return "kr";
  return "mixed";
}

export function forecastAssetMarket(assetClass: ForecastAssetClass): Exclude<AnalysisMarket, "mixed"> {
  return ({
    korea_stock: "kr",
    united_states_stock: "us",
    equity_future: "securities_futures",
    index_future: "securities_futures",
    crypto_spot: "coin",
    crypto_perpetual: "crypto_futures",
  } as const)[assetClass];
}
