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

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::mcp_client::{authorize_outbound_argument, sanitize_and_tag_inbound_output};
use crate::workbench::{apply_worker_rlimits, isolated_worker_command};
use crate::{DataSensitivity, WorkerLimits, WorkspaceWriteMode};

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

/// **ADR-0008 Faz 3 canlı taşıma.** Bir dış MCP sunucusunu F4 sandbox'ında (`isolated_worker_command`
/// — ağ kapalı, overlay dosya sistemi, rlimit/cgroup) başlatır ve stdin/stdout üzerinden satır
/// JSON-RPC konuşur. `McpEgressSession`'ın taşıma trait'ini gerçekler; oturumun tüm güvenlik boru
/// hattı (dışarı/içeri filtreler) bunun ÜSTÜNDE çalışır, davranış değişmez.
///
/// **Ağ:** F4 sandbox'ı ağı her zaman kapatır (`WorkerNetwork::Denied`), yani yalnız yerel (ağsız)
/// sunucular buradan koşar — `network_allowed` sunucular ile uzak MCP taşımaları F10.
///
/// **stderr:** sunucunun stderr'i güvenilmezdir; şimdilik bilinçli olarak atılır (`Stdio::null`) —
/// yakalanıp loglanırsa ADR-0008'in "stderr ayrı sızıntı kanalı" uyarısı gereği aynı güvenilmez-veri
/// işleminden geçmesi gerekir; o, ayrı bir iyileştirme.
///
/// **Dürüst sınır:** gerçek `isolated_worker_command` (bwrap + systemd-run) bu geliştirme ortamında
/// tam koşamaz; F4'ün kendisi gibi bu sarmalama gerçek makinede doğrulanır. Aşağıdaki test, taşımanın
/// G/Ç mantığını (yaz→oku→zaman aşımı) gerçek bir alt-süreçle (test dalı bwrap'ı atlar) kanıtlar.
pub struct SandboxedStdioTransport {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<String>,
    request_timeout: Duration,
}

impl SandboxedStdioTransport {
    /// Sunucuyu başlatır. `jail_root`: sunucuyu hapsetmek için ayrılmış bir dizin (yazıları overlay
    /// ile kaybolur, gerçek workspace'e dokunmaz). `request_timeout`: istek-başına yanıt beklemesi.
    pub fn launch(
        jail_root: &Path,
        program: &Path,
        args: &[&str],
        limits: &WorkerLimits,
        request_timeout: Duration,
    ) -> Result<Self, String> {
        let mut command = isolated_worker_command(
            jail_root,
            None,
            &[],
            program,
            args,
            limits,
            WorkspaceWriteMode::Overlay,
        )?;
        apply_worker_rlimits(&mut command, limits);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| format!("MCP sunucusu başlatılamadı: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "MCP sunucusunun stdin'i alınamadı".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP sunucusunun stdout'u alınamadı".to_string())?;
        // Okuma ayrı bir thread'de: böylece `request` bir yanıtı `recv_timeout` ile bekleyebilir —
        // asılı bir sunucu JARVIS'i süresiz bloke etmez (F9 timeout disiplini). Sunucu çıkarsa
        // kanal kapanır ve `request` "bağlantı kapandı" döner.
        let (sender, responses) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            responses,
            request_timeout,
        })
    }
}

impl McpRpcTransport for SandboxedStdioTransport {
    fn request(&mut self, request_line: &str) -> Result<String, String> {
        writeln!(self.stdin, "{request_line}")
            .map_err(|error| format!("MCP sunucusuna yazılamadı: {error}"))?;
        self.stdin
            .flush()
            .map_err(|error| format!("MCP sunucusuna gönderilemedi: {error}"))?;
        match self.responses.recv_timeout(self.request_timeout) {
            Ok(line) => Ok(line),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = self.child.kill();
                Err("MCP sunucusu zaman aşımına uğradı (yanıt yok) — süreç öldürüldü".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("MCP sunucusu bağlantıyı kapattı".into())
            }
        }
    }
}

impl Drop for SandboxedStdioTransport {
    fn drop(&mut self) {
        // Oturum bitince süreci kesin öldür — yetim/asılı bir dış sunucu bırakma (F9 disiplini).
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
#[path = "mcp_egress_tests.rs"]
mod tests;
