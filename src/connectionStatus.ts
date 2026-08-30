export type ConnectionTone = "connected" | "partial" | "disconnected" | "neutral";

export const connectionTone = (connected: boolean, configured = false): ConnectionTone => {
  if (connected) return "connected";
  if (configured) return "partial";
  return "disconnected";
};

export const CONNECTION_LEGEND: Array<{ tone: Exclude<ConnectionTone, "neutral">; label: string; description: string }> = [
  { tone: "connected", label: "연결 완료", description: "바로 사용할 수 있음" },
  { tone: "partial", label: "확인 필요", description: "정보는 저장됐지만 로그인·권한·왕복 확인이 남음" },
  { tone: "disconnected", label: "미연결", description: "자격정보가 없거나 아직 지원하지 않음" },
];
