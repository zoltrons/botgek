# Telemetri

## Görev

Süreç içi sayaçlar ve gecikme histogramları. Panelin `/api/metrics` ucu ve MCP araçları bunları okur. Dış
bir metrik sistemi yoktur.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/telemetry.rs` | Sayaçlar, histogramlar, fazlar, REST rota sınıfları. |
| `src/benchmarks.rs` | Çevrimdışı kart üretimi karşılaştırması (`#[ignore]`). |
| `src/converters/builtin/twitter/benchmarks.rs` | X veri süresi ölçümü (`#[ignore]`). |

## Nasıl çalışır

Tasarım ilkeleri:

- Yeni bağımlılık yok. Sabit kovalı histogramlar ve atomik sayaçlar yeterlidir.
- Etiket kümesi sabittir. Kanal ya da webhook kimliği etiket olmaz, kardinalite büyümez.
- Sıcak yolda kilit yoktur. Her gözlem birkaç atomik işlemdir. JSON görüntüsü yalnızca istenince üretilir.
- Sayaçlar yeniden başlatmada sıfırlanır.

Parçalar:

- **Platform sayaçları:** `record(platform, ok, elapsed)`. Platform listesi `PLATFORMS` sabitidir (12 platform).
- **Fazlar:** `Phase` enum'u: `QueueWait`, `Fetch`, `Media`, `Render`, `WebhookWait`, `WebhookResolve`,
  `WebhookSend`, `Send`, `Delete`, `Reaction`, `Total`, `Preview`. `PhaseTimer::new(phase)` düşünce süreyi
  kaydeder.
- **REST:** `RouteClass` ile altı rota sınıfı. `rest_request`, `rest_attempt`, `rest_retry`,
  `rest_rate_limit_wait` kayıtları ve belirli durum kodları (400, 401, 403, 404, 408, 429, 500, 502, 503, 504)
  için ayrı sayaç.
- **Kuyruk:** `CONVERSION_QUEUE_ENQUEUED`, `COMPLETED`, `SATURATED`, `DEPTH`.
- **Diğer:** `DISCORD_READY`, `CACHE_HITS`, `OVERLOADS`, gateway olay sayacı (`GatewayEvent`: Ready, Message, Reaction, Interaction, ShardStage, Other).
- **Histogram:** 12 kovalı `Histogram`, son 64 örnek için tam yüzdelik hesabı yapar.

## Değişmezler

- Yeni platform eklenince `PLATFORMS` dizisinin uzunluğu ve ona bağlı sabit boyutlu diziler güncellenmeli.
- Yeni `Phase` eklenirse `ALL`, `name` ve `PHASES` boyutu birlikte değişir.
- Etiket olarak kullanıcıdan gelen ya da sınırsız bir değer kullanmayın.

## Değiştirirken

Yeni bir ölçüm noktası: `PhaseTimer` ya da `record_phase` kullanın, MCP `bot_metrics` ve `bot_diagnostics`
çıktısına snapshot üzerinden otomatik girer.

## Testler

```sh
cargo test --locked telemetry
cargo test --release --locked offline_card_baseline -- --ignored --nocapture
cargo test --locked twitter::benchmarks -- --ignored --nocapture
```
