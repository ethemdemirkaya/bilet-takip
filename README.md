# Bilet Takip

Kayseri'deki konser, tiyatro ve stand-up etkinliklerinin bilet fiyatlarını **Bubilet**, **Biletinial** ve **Biletix**'ten takip eder. Telegram'dan linkiyle birlikte şu durumlarda haber verir:

- 🔥 site bir etkinliğin herhangi bir kategorisine indirim koyunca (üstü çizili fiyat; şimdilik Bubilet)
- 📉 bir kategorinin fiyatı düşünce (varsayılan: en az %5 ya da en az 50 ₺)
- 🎟️ tükenen bilet tekrar satışa çıkınca

Aynı etkinlik birden fazla sitede satılıyorsa mesajda diğer sitelerdeki fiyatlar da gösterilir.

> Bilet **almaz**. Sadece herkese açık sayfaları okur, istekler arasında bekler.

## Nasıl çalışır

GitHub Actions her 20 dakikada bir programı çalıştırır. Program:

1. siteleri tarar,
2. sonucu bir önceki taramayla karşılaştırır (`state/state.json`),
3. gerekiyorsa Telegram'a mesaj atar,
4. bir sonraki çalıştırmaya kadar (~17 dk) Telegram'ı dinleyip komutlara anında cevap verir,
5. durumu repoya geri kaydeder.

Sunucu gerekmez, ücretsizdir.

## Kurulum

1. Bu repoyu GitHub'a gönder. Public repolarda Actions dakikası sınırsızdır.
2. **Settings → Secrets and variables → Actions** altına iki secret ekle:
   - `TELEGRAM_BOT_TOKEN`: @BotFather'ın verdiği token
   - `TELEGRAM_CHAT_ID`: senin chat ID'n
3. **Actions** sekmesinde "Bilet Takip" → **Run workflow** ile ilk çalıştırmayı başlat.

İlk tarama sadece mevcut fiyatları kaydeder ve "Takip başladı" mesajı gönderir. Sonraki taramalardan itibaren değişiklikleri bildirir.

## Telegram komutları

| Komut | Ne yapar |
|---|---|
| `/indirim` | Şu an indirimdeki etkinlikler |
| `/liste` | Yaklaşan etkinlikler ve en ucuz fiyatlar |
| `/ara karsu` | Etkinlik arama |
| `/tara` | Beklemeden hemen tarar |
| `/durum` | Kaynakların son durumu |
| `/esik 10` | Bildirim eşiğini %10 yapar |

Komutlara birkaç saniye içinde cevap verilir. İki çalıştırma arasındaki kısa geçişte cevap bir iki dakika gecikebilir.

## PC'de ya da sunucuda çalıştırma

`bilet-takip.exe`'yi `config.toml` ve `.env` ile aynı klasöre koyup `baslat.bat`'a çift tıkla. Bu dosya programı `--loop 20` ile çalıştırır: Telegram'ı sürekli dinler, siteleri 20 dakikada bir tarar, pencere kapanana kadar çalışır.

> ⚠️ PC'de çalıştırırken GitHub Actions'ı **kapat** (Actions → Bilet Takip → ⋯ → Disable workflow). İkisi aynı anda çalışırsa bildirimler iki kez gelir ve Telegram iki dinleyiciye birden izin vermez.

Linux sunucuda: `cargo build --release`, sonra `./target/release/bilet-takip --loop 20` komutunu systemd servisi ya da `tmux` içinde çalıştır.

## Geliştirme

`.env` dosyası oluştur (repoya **eklenmez**):

```
TELEGRAM_BOT_TOKEN=...
TELEGRAM_CHAT_ID=...
```

```sh
cargo test                         # kayıtlı örnek sayfalarla testler
cargo run --release -- --dry-run   # Telegram'a göndermeden mesajları konsola yazar
cargo run --release -- chat-id     # bota yazanların chat ID'lerini gösterir
cargo run --release -- --loop 20   # sürekli mod
```

Ayarlar (şehir, eşikler, açık/kapalı kaynaklar, hangi bildirimlerin gideceği) `config.toml` dosyasında. Yeni etkinlik bildirimleri varsayılan olarak kapalı; açmak için `new_event = true`.

## Kaynaklar nasıl okunuyor

| Site | Yöntem |
|---|---|
| Bubilet | Şehir sayfasındaki Next.js verisinden etkinlik listesi → etkinlik başına seanslar → seans başına bilet kategorileri (eski ve indirimli fiyatlarıyla) |
| Biletinial | Kategori listesi → etkinlik sayfası → şehirdeki seansların kategori fiyatları |
| Biletix | Solr arama servisi → etkinlik başına kategori fiyatları |

Bir site yapısını değiştirirse o kaynak hata verir, diğerleri çalışmaya devam eder. Aynı kaynak 3 çalıştırma üst üste hata verirse Telegram'dan uyarı gelir. `fixtures/` klasöründeki örnek sayfalar parser testlerinde kullanılır.

**Bilinen sınırlar:** Biletix bazı etkinliklerin fiyatını internet kanalına açmıyor ("2 Al 1 Öde" gibi). Bu etkinliklerde fiyat düşüşü takip edilemez, yeni etkinlik ve tekrar satışa çıkma bildirimleri yine çalışır. GitHub'ın zamanlayıcısı yoğun saatlerde birkaç dakika gecikebilir.
