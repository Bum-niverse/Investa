export type AnalysisSnapshotCommand = "toss_analysis_snapshot" | "upbit_analysis_snapshot" | "binance_perpetual_analysis_snapshot" | "kis_futures_analysis_snapshot";

export function selectAnalysisSnapshotCommand(request: string): AnalysisSnapshotCommand {
  const normalized = request.trim().toUpperCase();
  const securitiesFuture = /증권\s*선물|지수\s*선물|주식\s*선물|KOSPI\s*200\s*선물|KIS\s*선물|\b[0-9]{3}[A-Z][0-9]{2}\b/.test(normalized);
  if (securitiesFuture) return "kis_futures_analysis_snapshot";
  const perpetual = /코인\s*선물|무기한|PERPETUAL|\bPERP\b|[A-Z0-9]{2,}USDT\b/.test(normalized);
  if (perpetual) return "binance_perpetual_analysis_snapshot";
  const cryptoSpot = /KRW-[A-Z0-9]+|비트코인|이더리움|리플|\bBTC\b|\bETH\b|\bXRP\b|코인\s*현물/.test(normalized);
  return cryptoSpot ? "upbit_analysis_snapshot" : "toss_analysis_snapshot";
}
