# APK Engine Toolkit

APK Explorer & Editor bileşenleri ve Running Services Monitor ayrıştırıcılarından hazırlanmış ortak CLI ve MCP araç takımı. Sürüm 0.2.0, toplam 18 araç. Terminal komutları ve MCP araçları aynı motor çekirdeğini, girdi şemalarını ve JSON sonuçlarını kullanır.

## Motorlar

| Motor | Sürüm | Kullanım |
|---|---|---|
| aXML JVM core | b71030bd-jvm1 | Binary XML encode/decode ve ARSC okuma |
| Smali / Baksmali / dexlib2 | 2.5.2 | DEX sınıfları ve smali dönüşümü |
| apksig | 9.4.0 | APK imzalama ve doğrulama |
| Zip4j | 2.11.6 | Arşiv listeleme, çıkarma ve oluşturma |
| zipalign-java | 1.2.1 | 4 bayt ve 16 KiB ZIP girdi hizalama |
| RSM ProcessParser | 772e266f-java1 | Servis, LRU süreç ve sistem RAM bilgilerini birleştirme |
| RSM MemInfoParser | 772e266f-java1 | Sürece ait PSS/RSS, heap, nesne ve tahsis bilgileri |

Kaynak seçimleri APK Explorer & Editor v0.36 incelemesine dayanır. zipalign-java 1.2.2 gerçek hizalama testini geçmediği için değiştirilmemiş 1.2.1 kullanılır. aXML portu Android host yardımcılarını ayırır, gerçek KXml2 parser kullanır ve API36.1 Android attribute tablosunu içerir. Ayrı motor paketleri kullanıcının `engine_library` koleksiyonundadır.

## CLI

Dağıtım paketindeki `bin/apk-engine` komutu:

```sh
apk-engine --help
apk-engine tools
apk-engine inspect-apk --schema
apk-engine inspect-apk '{"path":"/absolute/app.apk"}'
apk-engine list-entries '{"path":"/absolute/app.apk","prefix":"lib/"}'
apk-engine decode-xml '{"path":"/absolute/app.apk","entry":"AndroidManifest.xml"}'
apk-engine list-dex-classes '{"path":"/absolute/app.apk","limit":50}'
apk-engine extract-entries '{"path":"/absolute/app.apk","output_dir":"/absolute/extracted","entries":["classes.dex"]}'
apk-engine inspect-apk request.json
apk-engine inspect-apk - < request.json
```

Diğer komutlar: `create-archive`, `encode-xml`, `inspect-resources`, `disassemble-dex`, `assemble-dex`, `align-apk`, `check-alignment`, `sign-apk`, `verify-apk`. Dosya üreten araçlar açık çıktı yolu alır; var olan dosyayı değiştirmek için `overwrite:true` gerekir. Çıkarma ve smali çıktıları boş klasör kullanır.

İşlem sırası: değişiklikleri paketle, hizala, imzala, doğrula. İmza için kendi PKCS12/JKS dosyanı ve alias adını ver; parola varsayılan olarak `APK_ENGINE_KEYSTORE_PASSWORD` ortam değişkeninden okunur.

```sh
apk-engine align-apk '{"path":"/absolute/unsigned.apk","output":"/absolute/aligned.apk"}'
apk-engine sign-apk '{"path":"/absolute/aligned.apk","output":"/absolute/signed.apk","keystore":"/absolute/key.p12","alias":"release"}'
apk-engine verify-apk '{"path":"/absolute/signed.apk"}'
```

CLI başarılı çağrılarda `apk-engine.result/1` JSON zarfı ve exit0 üretir. Girdi/işlem hataları JSON hata ve exit1 üretir. İmza doğrulama sonucu `result.verified` alanındadır; `false` geçersiz imzayı bildirir.

## Android servis ve RAM araçları

```sh
apk-engine inspect-android-processes '{"package":"com.termux"}'
apk-engine inspect-android-meminfo '{"package":"com.termux"}'
apk-engine parse-process-snapshot '{"services_path":"/absolute/services.txt","lru_path":"/absolute/lru.txt","meminfo_path":"/absolute/meminfo.txt"}'
apk-engine parse-meminfo '{"path":"/absolute/app-meminfo.txt"}'
```

`inspect-android-processes` servis, LRU ve genel RAM çıktısını mevcut `phone-adb` bağlantısıyla toplar; `inspect-android-meminfo` seçilen paketin ayrıntılı bellek bilgisini okur. Dosyadan ayrıştıran araçlar ADB bağlantısı gerektirmez. Eksik ölçümler `null`, ölçülen sıfırlar `0` olarak korunur; birimler sonuç içinde belirtilir. Servis bağlantıları Android Binder bağlantılarıdır.

Varsayılan ADB komutu `$HOME/.local/bin/phone-adb` yoludur. Farklı bir bağlantı için `APK_ENGINE_ADB_COMMAND_JSON` ortam değişkenine komut ve sabit argümanlardan oluşan JSON dizisi verilir; örneğin `["/absolute/adb","-s","device-serial"]`. Native ayrıştırıcılar ELF içine gömülüdür; canlı toplama mevcut ADB aracını çalıştırır. Ayrıntılı meminfo ham metnini de almak için `raw_output:true` kullanılır. Çok süreçli çıktılarda `apk-engine.meminfo-batch/1` altında `processes` listesi döner; her süreç kendi PID ve ölçümlerini taşır. Süreç listesindeki paket filtresi sistem RAM özetinin cihaz geneli olmasını değiştirmez. LRU satırlarında `oomAdjustment` ve `processStateLabel` alanları açık anlamlarıyla sunulur; eski RSM alan adları da korunur.

RSM kaynak sürümü `v1.0.27+26`, commit `772e266fb7dd95c76a64b74d10b33af8b95122ab` olarak sabittir. Özgün Dart kaynakları iki bağımsız motor paketinde korunur; bağımlılıksız Java17 uyarlamaları native ELF içine derlenir. Android Intent modülü ayrı `engine_library/android-intent` paketindedir.

## MCP

```sh
apk-engine --mcp
```

Newline-delimited JSON-RPC stdio kullanılır. `initialize`, `notifications/initialized`, `ping`, `tools/list` ve `tools/call` uygulanır. Protokol sürümleri 2024-11-05, 2025-03-26, 2025-06-18 ve 2025-11-25 desteklenir. CLI'daki tireli adların MCP karşılığı alt çizgilidir: `inspect-apk` → `inspect_apk`. Sonuç hem `structuredContent` hem JSON metin içerik olarak verilir. Motor günlükleri stderr'e gider.

MCP istemci kaydı:

```json
{
  "mcpServers": {
    "apk-engine": {
      "command": "/absolute/bin/apk-engine",
      "args": ["--mcp"]
    }
  }
}
```

## Derleme ve doğrulama

`build.py --library /absolute/engine_library` Java17 uyumlu `dist/apk-engine.jar` ara çıktısını üretir. `dist/build.json` kaynak ve bağımlılık SHA256 kayıtlarını tutar. Ara çıktı `java -jar dist/apk-engine.jar` ile geliştirme/test amacıyla çalıştırılabilir.

Native workflow BellSoft Native Image ARM64 musl toolchain'i kullanır. Hedef gerçek statik Linux AArch64 ELF'tir; haricî JVM veya launcher içermez. ELF başlığı, dinamik yükleyici/bağımlılık bulunmaması ve ortak CLI/MCP testleri ayrı doğrulanır. Android/Termux çalıştırma kanıtı indirilen aynı artifact üzerinde ayrıca kaydedilir.

`tests/verify_toolkit.py` aynı testleri JVM ara çıktısına veya native executable'a uygular. ZIP/DEX/XML roundtrip, gerçek hizalama ofsetleri, APK imzası ve değiştirilmiş dosya reddi, JSON girdi biçimleri, MCP görüşmesi, RSM ayrıştırma örnekleri, gerçek alt süreç üzerinden kontrollü ADB yanıtları ve bütün araçların çağrılmasını kapsar. Geçici test anahtarları/dosyaları kaldırılır.

ARSC aracı aXML'in sunduğu kimlik, isim ve değerleri döndürür; complex bag ve bütün locale varyantlarını tam çözmez. Binary XML encoder desteklemediği metin düğümlerini ve bilinmeyen Android attribute isimlerini hata ile bildirir.

Özgün motor lisansları ve kaynak bildirimleri bağımlılık paketleri/CI girdileri içinde korunur. Bu toolkit'in değiştirilmiş aXML kaynakları ayrıca kaydedilir.
