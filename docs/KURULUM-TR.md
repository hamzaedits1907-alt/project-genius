# Genius Web kurulumu (basit Türkçe anlatım)

Bu sayfa teknik bilgi gerektirmeden Genius Web'i yayınlamak için ne yapacağını anlatır. Hepsi ücretsiz planlarla çalışır.

## 1. Supabase (hesaplar ve kayıtlı sohbetler)

1. https://supabase.com/dashboard adresine gir, zaten oluşturduğun **organization**'ı seç.
2. **New project** düğmesine bas, isim ver (ör. `genius`), bir veritabanı şifresi seç ve kaydet (şifreyi kimseye gösterme).
3. Proje açılınca soldan **SQL Editor**'ü aç. Bu klasördeki üç dosyayı sırayla içine yapıştırıp **Run**'a bas:
   - `supabase/migrations/20261019000000_genius_web.sql`
   - `supabase/migrations/20261019000100_genius_web_storage.sql`
   - `supabase/migrations/20261019000200_missions.sql`
4. Soldan **Project Settings → API** bölümüne gir. İki şeyi kopyalacaksın: **Project URL** ve **anon / publishable key**. (Başka hiçbir anahtarı — özellikle `service_role` — Netlify'a koyma. Bana da yapıştırma.)

## 2. Netlify

1. https://app.netlify.com → **Add new site → Import an existing project → GitHub** → `project-genius` deposunu seç.
2. **Branch to deploy** olarak **`genius-web-launch`** yaz. Diğer alanlara dokunma (ayarlar `netlify.toml` içinde).
3. **Site configuration → Environment variables** bölümünde iki değişken ekle:
   - `VITE_SUPABASE_URL` = Supabase'teki Project URL
   - `VITE_SUPABASE_ANON_KEY` = Supabase'teki anon / publishable key
4. **Deploy** et. Bitince sitenin adresini (ör. `https://xxxx.netlify.app`) not al.

## 3. Supabase'e site adresini söyle

Supabase → **Authentication → URL Configuration**:
- **Site URL**: `https://xxxx.netlify.app/app`
- **Redirect URLs**: aynı adres (`https://xxxx.netlify.app/app`). Yıldızlı (`*`) adres ekleme.

## 4. Kendini "owner" yap

1. Sitede `/app` adresine git, e-postanla kayıt ol, gelen onay e-postasındaki bağlantıya tıkla.
2. Supabase **SQL Editor**'de şunu çalıştır (e-postanı kendininkiyle değiştir):

```sql
insert into public.admin_users (user_id) select id from auth.users where email = 'SENIN-EPOSTAN';
```

3. Sayfayı yenile: sol altta adının yanında "owner" yazar, tıklayınca `/admin` paneli açılır.

## 5. Kendi AI anahtarını gir

Genius Web → **Settings → AI provider** → sağlayıcını seç → anahtarını yapıştır → **Test connection**. Anahtar sadece o tarayıcı sekmesinde durur, sekmeyi kapatınca silinir. NVIDIA tarayıcıdan doğrudan çağrılamaz; Settings'te "relay"e izin vermen gerekir (anahtarını saklamadan iletir).

## Sorun olursa

- "Accounts are not set up yet": Netlify'daki iki `VITE_...` değişkeni eksik; ekleyip yeniden deploy et.
- E-posta bağlantısı yanlış yere gidiyor: Supabase'teki Site URL / Redirect URL'i kontrol et.
- Admin paneli "Not authorized" diyor: 4. adımdaki SQL'i doğru e-postayla çalıştırdığından emin ol.
