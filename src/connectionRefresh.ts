export type ConnectionProbeState = "connected" | "attention" | "disconnected" | "failed";

export type ConnectionProbeResult = {
  id: string;
  label: string;
  state: ConnectionProbeState;
};

export type ConnectionRefreshSummary = {
  total: number;
  connected: number;
  attention: number;
  disconnected: number;
  failed: number;
  completedAtMs: number;
};

export function summarizeConnectionRefresh(
  results: ConnectionProbeResult[],
  completedAtMs: number,
): ConnectionRefreshSummary {
  return results.reduce<ConnectionRefreshSummary>((summary, result) => {
    summary[result.state] += 1;
    return summary;
  }, {
    total: results.length,
    connected: 0,
    attention: 0,
    disconnected: 0,
    failed: 0,
    completedAtMs,
  });
}
