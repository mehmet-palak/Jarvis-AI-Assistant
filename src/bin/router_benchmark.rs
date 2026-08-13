use std::path::PathBuf;
use std::time::Instant;

use jarvis_core::{route_with_provider, CapabilityRegistry, LlamaCliProvider, RouteSource};

struct Case {
    input: &'static str,
    expected: &'static str,
}

fn main() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let jarvis_root = project_root.parent().unwrap_or(&project_root);
    let executable = std::env::var_os("JARVIS_LLAMA_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| jarvis_root.join("third_party/llama.cpp/build-cpu/bin/llama-cli"));
    let model = std::env::var_os("JARVIS_MODEL_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| jarvis_root.join("models/Qwen3-8B-Q4_K_M.gguf"));
    let mut provider = LlamaCliProvider::cpu_default(executable, model);
    provider.threads = 4;
    provider.context = 512;
    provider.max_tokens = 8;
    let registry = CapabilityRegistry::baseline();
    let cases = [
        Case {
            input: "zaman nedir",
            expected: "system.time",
        },
        Case {
            input: "bilgisayarım iyi çalışıyor mu",
            expected: "system.health",
        },
        Case {
            input: "alışveriş listesi için not hazırla",
            expected: "note.create",
        },
    ];

    let mut passed = 0;
    let mut elapsed_ms = Vec::new();
    for case in &cases {
        let started = Instant::now();
        let route = route_with_provider(case.input, &registry, &provider);
        let elapsed = started.elapsed().as_millis();
        let ok = route.capability == case.expected && route.source == RouteSource::LocalModel;
        passed += usize::from(ok);
        elapsed_ms.push(elapsed);
        println!(
            "input={:?} expected={} actual={} source={:?} latency_ms={} result={}",
            case.input,
            case.expected,
            route.capability,
            route.source,
            elapsed,
            if ok { "PASS" } else { "FAIL" }
        );
    }
    let average = elapsed_ms.iter().sum::<u128>() / elapsed_ms.len() as u128;
    println!(
        "summary={}/{} average_latency_ms={}",
        passed,
        cases.len(),
        average
    );
    std::process::exit(if passed == cases.len() { 0 } else { 1 });
}
