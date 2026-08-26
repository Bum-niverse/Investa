const FIRESTORE_API = "https://firestore.googleapis.com/v1";
const METADATA_TOKEN_URL = "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

function encodeValue(value) {
  if (value instanceof Date) return { timestampValue: value.toISOString() };
  if (value === null) return { nullValue: null };
  if (typeof value === "string") return { stringValue: value };
  if (typeof value === "boolean") return { booleanValue: value };
  if (typeof value === "number" && Number.isInteger(value)) return { integerValue: String(value) };
  throw new Error("지원하지 않는 Firestore 값입니다.");
}

function decodeValue(value) {
  if ("stringValue" in value) return value.stringValue;
  if ("integerValue" in value) return Number(value.integerValue);
  if ("booleanValue" in value) return value.booleanValue;
  if ("nullValue" in value) return null;
  return undefined;
}

function documentFields(record) {
  return Object.fromEntries(Object.entries(record).map(([key, value]) => [key, encodeValue(value)]));
}

function parseDocument(document) {
  const record = Object.fromEntries(Object.entries(document.fields ?? {}).map(([key, value]) => [key, decodeValue(value)]));
  return { ...record, updateTime: document.updateTime, firestoreName: document.name };
}

export class FirestoreRepository {
  constructor({ projectId, database = "(default)", fetchImpl = fetch, accessToken, now = Date.now }) {
    this.fetch = fetchImpl;
    this.projectId = projectId;
    this.database = database;
    this.staticAccessToken = accessToken;
    this.now = now;
    this.token = null;
    this.tokenExpiresAt = 0;
    this.root = `${FIRESTORE_API}/projects/${encodeURIComponent(projectId)}/databases/${encodeURIComponent(database)}/documents`;
  }

  async accessToken() {
    if (this.staticAccessToken) return this.staticAccessToken;
    if (this.token && this.now() < this.tokenExpiresAt - 60_000) return this.token;
    const response = await this.fetch(METADATA_TOKEN_URL, { headers: { "Metadata-Flavor": "Google" } });
    if (!response.ok) throw new Error("Cloud 서비스 계정 토큰을 가져오지 못했습니다.");
    const payload = await response.json();
    this.token = payload.access_token;
    this.tokenExpiresAt = this.now() + Number(payload.expires_in ?? 300) * 1_000;
    return this.token;
  }

  async request(url, init = {}) {
    const token = await this.accessToken();
    const response = await this.fetch(url, {
      ...init,
      headers: { "content-type": "application/json", authorization: `Bearer ${token}`, ...(init.headers ?? {}) },
    });
    const text = await response.text();
    const payload = text ? JSON.parse(text) : null;
    if (!response.ok) {
      const error = new Error(`Firestore 요청 실패: ${response.status}`);
      error.status = response.status;
      error.payload = payload;
      throw error;
    }
    return payload;
  }

  async createOnce(collection, documentId, record) {
    const url = `${this.root}?collectionId=${encodeURIComponent(collection)}&documentId=${encodeURIComponent(documentId)}`;
    try {
      return parseDocument(await this.request(url, { method: "POST", body: JSON.stringify({ fields: documentFields(record) }) }));
    } catch (error) {
      if (error.status === 409) return null;
      throw error;
    }
  }

  async availableJobs(limit = 20) {
    const query = {
      structuredQuery: {
        from: [{ collectionId: "relay_jobs" }],
        where: { fieldFilter: { field: { fieldPath: "status" }, op: "IN", value: { arrayValue: { values: [{ stringValue: "queued" }, { stringValue: "leased" }] } } } },
        limit,
      },
    };
    const rows = await this.request(`${this.root}:runQuery`, { method: "POST", body: JSON.stringify(query) });
    return rows.filter((row) => row.document).map((row) => parseDocument(row.document));
  }

  async get(collection, documentId) {
    try {
      return parseDocument(await this.request(`${this.root}/${encodeURIComponent(collection)}/${encodeURIComponent(documentId)}`));
    } catch (error) {
      if (error.status === 404) return null;
      throw error;
    }
  }

  async patch(collection, documentId, record, expectedUpdateTime) {
    const masks = Object.keys(record).map((field) => `updateMask.fieldPaths=${encodeURIComponent(field)}`).join("&");
    const precondition = expectedUpdateTime ? `&currentDocument.updateTime=${encodeURIComponent(expectedUpdateTime)}` : "";
    const url = `${this.root}/${encodeURIComponent(collection)}/${encodeURIComponent(documentId)}?${masks}${precondition}`;
    return parseDocument(await this.request(url, { method: "PATCH", body: JSON.stringify({ fields: documentFields(record) }) }));
  }

  async claimOldest(deviceId, nowMs, leaseSeconds) {
    const jobs = (await this.availableJobs()).filter((job) => job.status === "queued" || Number(job.leaseExpiresAtMs ?? 0) <= nowMs);
    jobs.sort((left, right) => left.createdAtMs - right.createdAtMs || String(left.jobId).localeCompare(String(right.jobId)));
    for (const job of jobs) {
      try {
        return await this.patch("relay_jobs", job.jobId, {
          status: "leased",
          leasedBy: deviceId,
          leaseExpiresAtMs: nowMs + leaseSeconds * 1_000,
          updatedAtMs: nowMs,
        }, job.updateTime);
      } catch (error) {
        if (error.status !== 409 && error.status !== 412) throw error;
      }
    }
    return null;
  }
}
