# Genius Web ve masaüstü hesabı kurulumu (basit Türkçe anlatım)

Bu sayfa teknik bilgi gerektirmeden Genius Web'i yayınlamak ve masaüstü uygulamasında aynı hesapla giriş yapmak için ne yapacağını anlatır. Hepsi ücretsiz planlarla çalışır.

## 1. Supabase (hesaplar ve kayıtlı sohbetler)

1. https://supabase.com/dashboard adresine gir, zaten oluşturduğun **organization**'ı seç. (Yeni organization **açma**; mevcut projeyi kullan.)
2. Henüz proje yoksa **New project** düğmesine bas, isim ver (ör. `genius`), bir veritabanı şifresi seç ve kaydet (şifreyi kimseye gösterme).
3. Soldan **SQL Editor**'ü aç. `supabase/migrations` klasöründeki dosyaları **sırayla** içine yapıştırıp **Run**'a bas. Daha önce çalıştırdıklarını tekrar çalıştırma:
   - `20261019000000_genius_web.sql`
   - `20261019000100_genius_web_storage.sql`
   - `20261019000200_missions.sql`
   - `20261020000000_admin_advanced.sql` (gelişmiş yönetim paneli, bakım modu, duyurular, plan limitleri)
   - `20261021000000_sync.sql` (**yeni**: konuşmaların cihazlar arası eşitlenmesi)
   - `20261022000000_key_vault.sql` (**yeni**: API anahtarlarının hesapta şifreli saklanması)
   - `20261023000000_admin_access.sql` (**yeni**: herkese kapat/aç, erken erişim, davet, bekleme listesi, kullanıcıya özel limit)
4. Soldan **Project Settings → API** bölümüne gir. İki şeyi kopyalacaksın: **Project URL** ve **anon / publishable key**. (Başka hiçbir anahtarı — özellikle `service_role` / `secret` — hiçbir yere koyma. Bana da yapıştırma.)

## 2. E-posta doğrulamasını kapat (kayıt olunca hemen girilsin)

Bunu **kod değiştiremez**, Supabase panelinden senin yapman gerekir:

1. Supabase → **Authentication → Sign In / Providers** (eski panelde *Providers*) → **Email**.
2. **Confirm email** düğmesini **kapat** → **Save**.

Sonuç: Kayıt olan kişi doğrulama e-postası beklemeden hemen içeri girer (masaüstü uygulamada da, web'de de). Şifre sıfırlama e-postası çalışmaya devam eder.

> Güvenlik notu: Doğrulama kapalıyken herkes istediği e-postayla kayıt olabilir. Bu yüzden **yönetici yetkisi hiçbir zaman e-posta adresine bağlı değildir**; yetki sadece senin ele geçirdiğin kullanıcı kimliğine (UUID) verilir (4. adım).
> Spam kayıt olursa yönetim panelinde **Site controls → Close registrations** ile kayıtları durdurabilirsin.

## 3. Netlify

1. https://app.netlify.com → **Add new site → Import an existing project → GitHub** → `project-genius` deposunu seç (zaten yaptıysan sadece değişkenleri kontrol et).
2. **Branch to deploy** olarak **`genius-web-launch`** yaz. Diğer alanlara dokunma (ayarlar `netlify.toml` içinde).
3. **Site configuration → Environment variables** bölümünde iki değişken ekle:
   - `VITE_SUPABASE_URL` = Supabase'teki Project URL
   - `VITE_SUPABASE_ANON_KEY` = Supabase'teki anon / publishable key
4. **Deploy** et. Bitince sitenin adresini (ör. `https://xxxx.netlify.app`) not al.

## 4. Supabase'e site adresini söyle

Supabase → **Authentication → URL Configuration**:
- **Site URL**: `https://xxxx.netlify.app/app`
- **Redirect URLs**: aynı adres (`https://xxxx.netlify.app/app`). Yıldızlı (`*`) adres ekleme.

## 5. Kendini "owner" yap (tek seferlik, güvenli yol)

1. Sitede `/app` adresine git, e-posta + şifreyle kayıt ol (doğrulama kapalıysa direkt girersin).
2. Supabase → **Authentication → Users** listesinde kendi satırını bul, **User UID** değerini kopyala (uzun bir kimlik: `xxxxxxxx-xxxx-…`).
3. **SQL Editor**'de şunu çalıştır (UID'yi kendininkiyle değiştir):

```sql
insert into public.admin_users (user_id) values ('BURAYA-USER-UID');
```

4. Sayfayı yenile: sol altta adının yanında "owner" yazar, tıklayınca `/admin` paneli açılır.

Daha önce e-posta ile owner olduysan **hiçbir şey yapma**; mevcut kayıt korunur. Kontrol: `select a.user_id, u.email from public.admin_users a join auth.users u on u.id = a.user_id;`

## 6. Masaüstü uygulamada giriş

Masaüstünde giriş **isteğe bağlıdır** (Genius hesapsız da çalışır). **Settings → Account** bölümünde e-posta + şifre ile kayıt olunur / giriş yapılır; web'deki hesapla **aynıdır**.

İki yoldan biri gerekir (bir kez):

- **Önerilen (kullanıcılar hiçbir şey yapmasın):** GitHub'da depo → **Settings → Secrets and variables → Actions → Variables** sekmesi → **New repository variable**: `VITE_SUPABASE_URL` ve `VITE_SUPABASE_ANON_KEY` (ikisi de herkese açık değerlerdir, gizli değildir). Sonraki sürüm bu değerlerle derlenir.
- **Elle:** uygulamada Settings → Account → "Connect the Genius account service" kutusuna aynı iki değeri yapıştır. (Service-role / secret anahtarı uygulama reddeder.)

## 7. Kendi AI anahtarını gir (bir kere)

**NVIDIA anahtarı nasıl alınır (ücretsiz):** sitenin ana sayfasındaki "Get a free NVIDIA API key" bölümüne bak — özet: https://build.nvidia.com → giriş yap → profil → **API Keys** → **Generate API Key** → `nvapi-...` ile başlayan anahtarı kopyala.

- **Masaüstü:** Settings → AI provider → NVIDIA → anahtarı yapıştır → **Test connection**. Anahtar Windows Kimlik Bilgisi Yöneticisi'nde saklanır; uygulamayı kapatıp açsan da kalır. Silmek/değiştirmek için aynı ekrandaki **Remove** / yeni anahtar.
- **Web:** aynı ekran. **"Remember this key on this device"** kutusunu işaretlersen anahtar bu tarayıcıda **şifrelenmiş** saklanır, her ziyarette sormaz. İşaretlemezsen sadece o sekme açıkken durur. Anahtar **asla** Supabase'e / Genius sunucusuna gönderilmez, cihazlar arası eşitlenmez. Ortak bilgisayarda işaretleme.
- NVIDIA tarayıcıdan doğrudan çağrılamaz; Web'de Settings'te "relay"e izin vermen gerekir (anahtarını saklamadan iletir).

## Sorun olursa

- "Accounts are not set up yet": Netlify'daki iki `VITE_...` değişkeni eksik; ekleyip yeniden deploy et.
- Kayıt olunca "doğrulama e-postası gönderildi" diyor: 2. adımdaki **Confirm email** hâlâ açık.
- E-posta bağlantısı yanlış yere gidiyor: Supabase'teki Site URL / Redirect URL'i kontrol et.
- Admin paneli "Not authorized" diyor: 5. adımdaki SQL'i doğru UID ile çalıştırdığından emin ol.
- Masaüstünde "Connect the Genius account service" kutusu görünüyor: 6. adımdaki iki değeri gir.
