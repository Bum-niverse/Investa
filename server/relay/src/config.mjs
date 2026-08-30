const required = (env, name) => {
  const value = env[name]?.trim();
  if (!value) throw new Error(`필수 환경변수 ${name}이(가) 없습니다.`);
  return value;
};

const boundedInteger = (value, fallback, min, max) => {
  if (value == null || value === "") return fallback;
  const parsed = Number.parseInt(value, 10);
  if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) {
    throw new Error(`환경변수 정수 범위가 올바르지 않습니다: ${min}~${max}`);
  }
  return parsed;
};

export function loadConfig(env = process.env) {
  const allowedTelegramUserIds = new Set(
    required(env, "ALLOWED_TELEGRAM_USER_IDS")
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean),
  );
  if (
    allowedTelegramUserIds.size === 0 ||
    allowedTelegramUserIds.size > 10 ||
    [...allowedTelegramUserIds].some((value) => !/^-?\d{1,31}$/.test(value))
  ) {
    throw new Error("ALLOWED_TELEGRAM_USER_IDS는 숫자 ID 1~10개여야 합니다.");
  }

  const desktopSharedSecret = required(env, "DESKTOP_SHARED_SECRET");
  if (Buffer.byteLength(desktopSharedSecret) < 32) {
    throw new Error("DESKTOP_SHARED_SECRET은 최소 32바이트여야 합니다.");
  }

  return Object.freeze({
    projectId: required(env, "GOOGLE_CLOUD_PROJECT"),
    telegramBotToken: required(env, "TELEGRAM_BOT_TOKEN"),
    telegramWebhookSecret: required(env, "TELEGRAM_WEBHOOK_SECRET"),
    desktopSharedSecret,
    allowedTelegramUserIds,
    port: boundedInteger(env.PORT, 8080, 1, 65_535),
    maxBodyBytes: boundedInteger(env.MAX_BODY_BYTES, 16_384, 1_024, 65_536),
    maxClockSkewSeconds: boundedInteger(env.MAX_CLOCK_SKEW_SECONDS, 300, 30, 900),
    leaseSeconds: boundedInteger(env.LEASE_SECONDS, 90, 30, 600),
    maxRequestsPerMinute: boundedInteger(env.MAX_REQUESTS_PER_MINUTE, 60, 10, 600),
    jobRetentionHours: boundedInteger(env.JOB_RETENTION_HOURS, 24, 1, 168),
    firestoreDatabase: env.FIRESTORE_DATABASE?.trim() || "(default)",
  });
}
