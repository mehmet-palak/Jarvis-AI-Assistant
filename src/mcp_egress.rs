//! F8 MCP egress **oturum boru hattı** (ADR-0008 Faz 3, taşıma katmanı). Bir dış MCP sunucusuyla
//! JSON-RPC (satır-tabanlı) konuşurken güvenlik kararlarını doğru sırada uygular:
//! **dışarı filtresi → istek → yanıt ayrıştırma → içeri provenance etiketi.**
//!
//! Taşıma bir trait'in (`McpRpcTransport`) arkasında soyutlanmıştır — bu, güvenlik boru hattı ile
//! ağ/süreç ayrıntısı arasındaki **seam**'dir. Testler scripted (ve **düşmanca**) bir mock kullanır;
//! böylece boru hattının tamamı (kötü niyetli bir sunucunun injection/sır/aşırı-boyut/JSON-RPC-hata
//! denemesi dahil) gerçek bir alt-süreç olmadan, bu ortamın sandbox kısıtlarından bağımsız olarak
//! kanıtlanır.
//!
//! **Dürüst sınır (F7 deseni):** trait'in gerçek gerçekleştirmesi — sunucu sürecini F4 sandbox'ında
//! (`isolated_worker_command`) başlatıp uzun-ömürlü stdin/stdout üzerinden satır JSON-RPC konuşan
//! taşıma — henüz yazılmadı; bu, F4/F7'nin aynı "gerçek makinede doğrulanmalı" uyarısına tabi olan
//! canlı bir parça (bu geliştirme ortamı bazı sandbox özelliklerini kısıtlar). Boru hattının
//! karar/filtre mantığının tamamı bu seam'in ÜSTÜNDE, burada tam test edilmiştir; canlı taşıma
//! bağlandığında güvenlik davranışı değişmez, yalnız gerçek bir sunucuya ulaşır.

use serde_json::{json, Value};

use crate::mcp_client::{authorize_outbound_argument, sanitize_and_tag_inbound_output};
use crate::DataSensitivity;

/// Bir JSON-RPC istek satırı gönderip tek satır yanıt alan taşıma. Gerçek impl sandbox'lı alt-sürecin
/// stdin/stdout'u; test impl scripted.
pub trait McpRpcTransport {
    fn request(&mut self, request_line: &str) -> Result<String, String>;
}

/// Bir dış MCP sunucusuyla tek bir oturum. `server_id` yalnız içeri gelen çıktının provenance
/// etiketinde kullanılır. Bağlanma yetkisi (`authorize_mcp_connection`) bu oturumdan ÖNCE, çağıran
/// tarafça verilmiş olmalıdır — bu tip yalnız yetkili bir bağlantının veri boru hattıdır.
pub struct McpEgressSession<T: McpRpcTransport> {
    transport: T,
    server_id: String,
    next_id: u64,
}

impl<T: McpRpcTransport> McpEgressSession<T> {
    pub fn new(server_id: impl Into<String>, transport: T) -> Self {
        Self {
            transport,
            server_id: server_id.into(),
            next_id: 1,
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Bir JSON-RPC isteği kurar, taşımaya gönderir ve yanıtı ayrıştırır. `result` (Value) ya da
    /// tipli bir hata döner. JSON-RPC hata nesnesi de (ADR-0008: ayrı sızıntı kanalı) burada bir
    /// hataya çevrilir.
    fn call_method(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.allocate_id();
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response_line = self.transport.request(&request.to_string())?;
        let response: Value = serde_json::from_str(&response_line)
            .map_err(|error| format!("dış sunucu yanıtı JSON değil: {error}"))?;
        if let Some(error) = response.get("error") {
            // Hata mesajı da güvenilmezdir; ham göstermek yerine kısaltıp işaretle.
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("bilinmeyen hata");
            let capped: String = message.chars().take(300).collect();
            return Err(format!("dış araç hatası: {capped}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| "dış sunucu yanıtında result yok".to_string())
    }

    /// `tools/call` — güvenli boru hattı: (1) argüman **dışarı** filtresinden geçer (tavan + sır);
    /// (2) istek gönderilir; (3) yanıttaki metin çıkarılır; (4) **içeri** provenance etiketiyle
    /// sarılır ("talimat değil, veri"). Dönen string doğrudan model bağlamına konulabilecek,
    /// güvenli-sarılmış çıktıdır.
    pub fn call_tool(
        &mut self,
        tool_name: &str,
        argument: &str,
        argument_sensitivity: DataSensitivity,
        server_ceiling: DataSensitivity,
    ) -> Result<String, String> {
        // (1) Dışarı: tavan + sır kontrolü. Şüpheliyse hiç gönderme.
        authorize_outbound_argument(argument, argument_sensitivity, server_ceiling)?;

        // (2) İstek.
        let params = json!({"name": tool_name, "arguments": {"input": argument}});
        let result = self.call_method("tools/call", params)?;

        // (3) MCP `tools/call` sonucu: content dizisindeki text parçalarını birleştir.
        let raw_output = extract_tool_text(&result);

        // (4) İçeri: boyut + sır redaksiyonu + provenance zarfı.
        Ok(sanitize_and_tag_inbound_output(
            &self.server_id,
            &raw_output,
        ))
    }
}

/// MCP `tools/call` sonucundan (`{"content":[{"type":"text","text":...}], ...}`) metni çıkarır.
/// Beklenmeyen bir şekil boş string verir (çökmez) — güvenilmez veriyi ayrıştırırken hoşgörülü ol.
fn extract_tool_text(result: &Value) -> String {
    let Some(items) = result.get("content").and_then(Value::as_array) else {
        return String::new();
    };
    let mut out = String::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "mcp_egress_tests.rs"]
mod tests;
