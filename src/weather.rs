//! JARVIS'in tek gerçek internet erişimi gerektiren yeteneği (kullanıcı onayıyla, 16 Ağustos
//! 2026) — yalnız açılış karşılamasında güncel hava durumunu göstermek için. Hiçbir governed
//! capability/task/policy yoluna girmez ve `CapabilityRegistry`'de bir kayıt değildir; model
//! buradan hiçbir zaman bir şey "çağıramaz", yalnız `Runtime::startup_briefing` başlangıç metnini
//! oluştururken bir kez okur. Bu, F3'ün "hiçbir capability ağ erişimi gerektirmiyor" testinin
//! (`no_baseline_capability_requires_network_access`) hâlâ doğru olduğu anlamına gelir — o test
//! yalnız `CapabilityRegistry` kayıtlarını kapsıyordu, ve hava durumu hiç bir kayıt değil.

use serde_json::Value;

pub trait WeatherProvider: std::fmt::Debug + Send + Sync {
    fn current_weather(&self) -> Result<WeatherSnapshot, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherSnapshot {
    pub location: String,
    pub temperature_celsius: f64,
    pub description: String,
}

/// Open-Meteo (open-meteo.com) — ücretsiz, API anahtarı gerektirmeyen, kayıt gerektirmeyen bir
/// hava durumu servisi. Kullanıcı onayıyla AccuWeather yerine tercih edildi — AccuWeather'ın
/// geliştirici API'si ücretli/kayıt gerektiriyor, bu proje "kayıt/API anahtarı olmayan, ücretsiz"
/// bir servisi tercih etti.
#[derive(Debug, Clone)]
pub struct OpenMeteoWeatherProvider {
    pub location_label: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timeout_seconds: u64,
}

impl OpenMeteoWeatherProvider {
    /// İstanbul, Ümraniye — kullanıcının belirttiği varsayılan konum (16 Ağustos 2026). Konum
    /// değişirse bu fonksiyonu güncellemek ya da doğrudan `OpenMeteoWeatherProvider { .. }`
    /// kurmak yeterli.
    pub fn istanbul_umraniye() -> Self {
        Self {
            location_label: "İstanbul, Ümraniye".into(),
            latitude: 41.0166,
            longitude: 29.1173,
            timeout_seconds: 5,
        }
    }
}

impl WeatherProvider for OpenMeteoWeatherProvider {
    fn current_weather(&self) -> Result<WeatherSnapshot, String> {
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current_weather=true",
            self.latitude, self.longitude
        );
        let response: Value = ureq::get(&url)
            .timeout(std::time::Duration::from_secs(self.timeout_seconds))
            .call()
            .map_err(|error| format!("hava durumu servisine erişilemedi: {error}"))?
            .into_json()
            .map_err(|error| format!("hava durumu yanıtı çözümlenemedi: {error}"))?;
        parse_open_meteo_response(&response, &self.location_label)
    }
}

/// Ayrı, saf bir fonksiyon (gerçek ağ çağrısından bağımsız) — böylece JSON ayrıştırma mantığı
/// gerçek bir ağ bağlantısı olmadan test edilebilir (`cargo test --offline` bunu asla ihlal
/// etmemeli).
fn parse_open_meteo_response(
    response: &Value,
    location_label: &str,
) -> Result<WeatherSnapshot, String> {
    let temperature_celsius = response
        .pointer("/current_weather/temperature")
        .and_then(Value::as_f64)
        .ok_or_else(|| "hava durumu yanıtında sıcaklık yok".to_string())?;
    let weather_code = response
        .pointer("/current_weather/weathercode")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    Ok(WeatherSnapshot {
        location: location_label.to_string(),
        temperature_celsius,
        description: describe_weather_code(weather_code).to_string(),
    })
}

/// Open-Meteo'nun WMO hava durumu kodlarının kısa Türkçe açıklaması — tam liste değil, en yaygın
/// olanları kapsıyor; bilinmeyen bir kod genel bir ifadeye düşer, hataya değil.
fn describe_weather_code(code: i64) -> &'static str {
    match code {
        0 => "açık",
        1 | 2 => "az bulutlu",
        3 => "kapalı",
        45 | 48 => "sisli",
        51 | 53 | 55 => "çisenti",
        61 | 63 | 65 => "yağmurlu",
        71 | 73 | 75 => "karlı",
        80..=82 => "sağanak yağmurlu",
        95 | 96 | 99 => "gök gürültülü fırtına",
        _ => "belirsiz",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_weather_code_covers_common_codes_and_falls_back_for_unknown() {
        assert_eq!(describe_weather_code(0), "açık");
        assert_eq!(describe_weather_code(61), "yağmurlu");
        assert_eq!(describe_weather_code(9999), "belirsiz");
    }

    /// Gerçek bir ağ bağlantısı olmadan — elle kurulmuş, gerçek Open-Meteo yanıtını taklit eden
    /// bir JSON ile ayrıştırma mantığını doğrular.
    #[test]
    fn parse_open_meteo_response_extracts_temperature_and_description() {
        let response = serde_json::json!({
            "latitude": 41.0,
            "longitude": 29.1,
            "current_weather": {
                "temperature": 24.3,
                "windspeed": 10.2,
                "weathercode": 0,
                "time": "2026-08-16T12:00"
            }
        });
        let snapshot = parse_open_meteo_response(&response, "İstanbul, Ümraniye")
            .expect("valid response parses");
        assert_eq!(snapshot.location, "İstanbul, Ümraniye");
        assert_eq!(snapshot.temperature_celsius, 24.3);
        assert_eq!(snapshot.description, "açık");
    }

    #[test]
    fn parse_open_meteo_response_rejects_a_response_with_no_temperature() {
        let response = serde_json::json!({ "current_weather": {} });
        assert!(parse_open_meteo_response(&response, "test").is_err());
    }

    #[test]
    fn parse_open_meteo_response_falls_back_gracefully_for_an_unknown_weather_code() {
        let response = serde_json::json!({
            "current_weather": { "temperature": 10.0, "weathercode": 123456 }
        });
        let snapshot = parse_open_meteo_response(&response, "test").expect("still parses");
        assert_eq!(snapshot.description, "belirsiz");
    }
}
