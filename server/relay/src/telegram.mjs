export class TelegramClient {
  constructor({ botToken, fetchImpl = fetch }) {
    this.fetch = fetchImpl;
    this.baseUrl = `https://api.telegram.org/bot${botToken}`;
  }

  async sendMessage(chatId, text) {
    const response = await this.fetch(`${this.baseUrl}/sendMessage`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ chat_id: chatId, text: String(text).slice(0, 3_800), disable_web_page_preview: true }),
    });
    if (!response.ok) throw new Error(`Telegram 회신 실패: ${response.status}`);
  }
}
