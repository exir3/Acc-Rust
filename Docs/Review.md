# گزارش جامع بررسی کد (بک‌اند Rust + فرانت‌اند)

پس از بررسی دقیق فایل‌های `db.rs`، `vouchers.rs`، `excel.rs`، `report.rs`، `lib.rs`، `app.js`، `voucher.js`، `account-definition.js`، `account-groups.js` و CSS/HTML، موارد زیر را به ترتیب اولویت گزارش می‌کنم.

---

## 🔴 موارد بحرانی (ممکن است باعث از دست رفتن داده یا رفتار نادرست شود)

### 1) نسخهٔ schema در `SCHEMA` هاردکد شده است
در `db.rs` داخل ثابت `SCHEMA`:
```sql
INSERT OR IGNORE INTO app_meta(key, value) VALUES ('schema_version', '3');
```
اگر روزی `CURRENT_SCHEMA_VERSION` تغییر کند، این مقدار باید دستی هم‌زمان عوض شود؛ در غیر این صورت یک DB تازه با نسخهٔ قدیمی علامت می‌خورد و `apply_schema_migrations` دوباره migration را اجرا می‌کند.

**راهکار:** مقدار را از ثابت بسازید:
```rust
let schema = SCHEMA.replace(
    "VALUES ('schema_version', '3')",
    &format!("VALUES ('schema_version', '{}')", CURRENT_SCHEMA_VERSION),
);
```
یا بهتر، `INSERT OR IGNORE` را از `SCHEMA` حذف کرده و در `initialize` بعد از اجرای `SCHEMA` این کار را با پارامتر bind انجام دهید:
```rust
conn.execute(
    "INSERT OR IGNORE INTO app_meta(key, value) VALUES ('schema_version', ?1)",
    params![CURRENT_SCHEMA_VERSION.to_string()],
)?;
```

### 2) استفادهٔ غیرضروری از `unchecked_transaction` در `save_voucher_inner`
`with_conn` به closure یک `&mut Connection` می‌دهد، اما `save_voucher_inner` (و `delete_account_group` و `seed_*`) از `unchecked_transaction` استفاده می‌کنند که نسخهٔ بدون بررسی `&Connection` است. مزیت `Connection::transaction()` این است که اگر از قبل تراکنشی باز باشد، در همان نقطهٔ Rust خطا می‌دهد نه در SQLite. همچنین از nested transaction ناخواسته جلوگیری می‌کند.

**راهکار:**
```rust
let tx = conn.transaction().map_err(db_error)?;
```
(همین تغییر در `delete_account_group`، `seed_default_groups`، `seed_default_accounts`، `seed_default_settings` و `apply_schema_migrations` هم قابل اعمال است. در `memory_database` که `&Connection` دارد، `unchecked_transaction` لازم است.)

### 3) `get_trial_balance` کل خطوط را در Rust تجمیع می‌کند
الگوی فعلی: `LEFT JOIN voucher_lines + vouchers` بدون `GROUP BY`، سپس در Rust در `BTreeMap` تجمیع می‌شود. برای چند هزار سند و ده‌ها هزار ردیف، هر بار گزارش گرفتن کند می‌شود و چون فقط یک `Mutex<Connection>` وجود دارد، تمام عملیات دیگر (حتی ذخیرهٔ سند) در این مدت بلاک می‌شوند.

**راهکار پیشنهادی:** تجمیع را به SQL منتقل کنید:
```sql
SELECT a.code, a.name, g.name, a.level,
       COALESCE(SUM(CASE WHEN ?1 <> '' AND v.date < ?1 THEN l.debit END), 0)  AS od,
       COALESCE(SUM(CASE WHEN ?1 <> '' AND v.date < ?1 THEN l.credit END), 0) AS oc,
       COALESCE(SUM(CASE WHEN (?1 = '' OR v.date >= ?1) AND (?2 = '' OR v.date <= ?2)
                         THEN l.debit END), 0)  AS pd,
       COALESCE(SUM(CASE WHEN (?1 = '' OR v.date >= ?1) AND (?2 = '' OR v.date <= ?2)
                         THEN l.credit END), 0) AS pc
  FROM accounts a
  JOIN account_groups g ON g.code = a.group_code
  LEFT JOIN voucher_lines l ON l.account_code = a.code
  LEFT JOIN vouchers v      ON v.id = l.voucher_id
 GROUP BY a.code
 ORDER BY a.code;
```
(همین کار برای `get_ledger` هم می‌تواند با CTE و `SUM` انجام شود.)

### 4) `list_vouchers` و `get_journal` بدون LIMIT
`list_vouchers` همهٔ اسناد مطابق فیلتر را برمی‌گرداند و `renderVouchers` همه را در یک `<table>` می‌سازد. با ۵٬۰۰۰ سند، رابط کاربری چند ثانیه فریز می‌شود (مشابه `Book1.xlsx` که ۵٬۲۸۲ سند دارد).

**راهکار:** پارامترهای `offset`/`limit` به `list_vouchers` و `get_journal` اضافه کنید و در UI صفحه‌بندی نمایش دهید. حداقل یک دکمهٔ «بارگذاری بیشتر» یا نمایش ۲۰۰ ردیف اول با شمارش کل.

### 5) مقدار `MAX_VOUCHER_LINES` در دو لایه همگام نیست
- بک‌اند: `MAX_LINES = 1000`
- فرانت `voucher.js`: `MAX_VOUCHER_LINES = 1000`

مقدار یکسان است، ولی اگر فردا تغییر کند، هم‌زمان نبودن باعث پیام خطای گیج‌کننده می‌شود. بهتر است فرانت از پاسخ بک‌اند بخواند (مثلاً در `get_settings` یک فیلد `limits` برگردانده شود).

---

## 🟠 موارد متوسط (باگ یا محدودیت محسوس)

### 6) نمایش «قابل ثبت نیست» برای حساب‌های سطح ۴ قدیمی در فرم سند
در `voucher.js` تابع `updateRowAccount`:
```javascript
if (account && account.isPostable) input.classList.remove('is-invalid');
else if (!code) input.classList.remove('is-invalid');
else if (final) input.classList.add('is-invalid');
```
`list_accounts` برای سطح > ۳ مقدار `is_postable = false` برمی‌گرداند. پس هر حساب سطح ۴ (legacy) که در سند قدیمی ذخیره شده، هنگام خروج از فیلد قرمز می‌شود؛ در حالی که `validateDraft` آن را مجاز می‌داند:
```javascript
const legacyAccountAlreadyOnThisVoucher = Number(account.level) > 3
  && (state.voucher?.lines || []).some(...);
if (!account.isPostable && !legacyAccountAlreadyOnThisVoucher) { return '...'; }
```
نتیجه: کاربر قرمز می‌بیند اما ذخیره موفق است.

**راهکار:** در `updateRowAccount` هم همان شرط legacy را در نظر بگیرید:
```javascript
const legacyOk = Number(account.level) > 3
  && (state.voucher?.lines || []).some(l => digitsOnly(l.accountCode) === code);
if ((account.isPostable || legacyOk)) input.classList.remove('is-invalid');
else if (code && final) input.classList.add('is-invalid');
```

### 7) عدم تطابق `length` در JS با `chars().count()` در Rust
در `voucher.js`:
```javascript
if (String(line.description || '').length > 500) errors.push('شرح بیش از ۵۰۰ نویسه');
```
`String.length` در JS واحد UTF-16 می‌شمارد؛ برای متون فارسی یکسان است، اما برای ایموجی/کاراکترهای surrogate pair با `chars().count()` متفاوت می‌شود. بک‌اند ممکن است قبول کند و فرانت رد کند (یا برعکس).

**راهکار:**
```javascript
if ([...String(line.description || '')].length > 500) ...
```
(همین مورد در `parseVoucherClipboardRows` هم وجود دارد.)

### 8) اعتبارسنجی تاریخ در `validateDraft` ناقص است
```javascript
if (year < 1 || year > 3177 || month < 1 || month > 12 || day < 1 || day > 31) { ... }
```
روز ۳۱ برای ماه ۲ یا ۳۱ برای ماه‌های ۷-۱۲ معتبر نیست. فرانت این را رد نمی‌کند و اتکا به Rust است — که درست است، اما تجربهٔ کاربری بهتر است اگر پیام خطا قبل از رفت‌وبرگشت شبکه نمایش داده شود.

**راهکار:** قبل از فراخوانی `save_voucher` از `invoke('validate_voucher_date')` که الان در `saveVoucher` هست استفاده می‌شود؛ فقط کافی است در `validateDraft` برای روز/ماه محدودیت دقیق‌تر اضافه شود یا بلافاصله `validate_voucher_date` صدا زده شود.

### 9) عدم بازگردانی خودکار در صورت گیر افتادن در حالت `failed`
در `db.rs` اگر `restore_from` هم بازیابی و هم rollback شکست بخورد، `failed = Some(reason)` ست می‌شود و تا ری‌استارت برنامه هیچ عملیاتی مجاز نیست. کاربر هیچ دکمه‌ای برای «تلاش مجدد» ندارد.

**راهکار:** یک فرمان `retry_database` اضافه کنید که:
1. `failed` را پاک کند.
2. دوباره `initialize` را صدا بزند و `attach` کند.
3. در صورت موفقیت پیام موفقیت بدهد.

### 10) در bulk import، `general_code` بدون گروه معتبر ممکن است
`imported_group_code` برای کدهای ناشناخته پیش‌فرض "11" برمی‌گرداند. اگر گروه ۱۱ حذف شده باشد (کاربر می‌تواند گروه‌های غیرپیش‌فرض را حذف کند، ولی گروه ۱۱ جزو پیش‌فرض‌هاست)، `import_vouchers` در مرحلهٔ ساخت حساب خطای «گروه حساب ... پیدا نشد» می‌دهد — که درست است، ولی می‌توان قبل از شروع کار به کاربر گفت کدام کد کل به گروه نامعتبر اشاره می‌کند.

### 11) زمان‌های `updated_at` با دقت متفاوت
- `INSERT` از `DEFAULT (datetime('now', 'localtime'))` → دقت ثانیه
- `UPDATE` از `strftime('%Y-%m-%d %H:%M:%f', 'now', 'localtime')` → دقت میلی‌ثانیه

مقایسه درست کار می‌کند (رشته‌ای)، ولی اگر جایی الگویی مثل «آخرین تغییر در ثانیهٔ X» فرض شود، می‌شکند. پیشنهاد: در هر دو مسیر از یک فرمت استفاده کنید (`%Y-%m-%d %H:%M:%f`).

### 12) به‌روزرسانی گروه در `update_account_group` بدون قفل خوش‌بینانه
اگر دو کاربر هم‌زمان گروه را ویرایش کنند (البته در نسخهٔ دسکتاپ تک‌کاربره)، آخرین نوشتن برنده است. برای سازگاری با الگوی `save_voucher_if_unchanged`، می‌توانید `updated_at` را هم چک کنید. اولویت پایین.

### 13) `preview_voucher_reorganization` از `DefaultHasher` استفاده می‌کند
`DefaultHasher::new()` در نسخه‌های مختلف std Rust خروجی متفاوت می‌دهد (SipHash با کلیدهای ثابت ولی الگوریتم ممکن است تغییر کند). چون توکن فقط در همان نشست (preview → apply) استفاده می‌شود، مشکلی نیست. ولی اگر روزی توکن را در دیسک ذخیره کنید، می‌شکند. توضیح در کد کافی است.

### 14) `removeAll` در `account-definition.js`
حذف گروهی حساب‌ها بدون معامله‌ای اتمیک انجام می‌شود: هر حساب جداگانه `delete_account` صدا می‌زند. اگر وسط کار قطع شود، بخشی حذف شده و بخشی نه. **پیشنهاد:** یک فرمان `delete_accounts_bulk(ids: Vec<i64>)` در بک‌اند که همه را در یک تراکنش با گزارش موفق/ناموفق حذف کند.

### 15) `account-definition.js` — `newCode` می‌تواند `''` برگرداند
اگر همهٔ کدهای ممکن تا ۲۰ رقم پر شده باشند، `newCode` رشتهٔ خالی برمی‌گرداند و `startNew` بدون پیام، `code.value = ''` می‌گذارد و فوکوس روی `name` می‌رود. کاربر گیج می‌شود.

**راهکار:**
```javascript
const suggested = newCode(n, p);
if (!suggested) { say('کد آزادی در این سطح موجود نیست؛ کد را دستی وارد کنید.', 'error'); }
code.value = fa(suggested);
```

### 16) `syncVoucherRowCount` حداکثر ۱۰۰۰ ردیف
اگر سندی با ۱۰۰۰ ردیف داشته باشیم، فرم ۱۰۰۰ `<tr>` می‌سازد. هر `refreshFormState` (هر ورودی کاربر) روی همهٔ آن‌ها حلقه می‌زند. برای سند ۵۹۸ ردیفی (مثل Book1.xlsx) هنوز قابل تحمل است، ولی در تایپ سریع ممکن است لگ داشته باشد.

**راهکار:** `applyVoucherGridSearch` را با یک مجموعه از `Set<id>` و `requestAnimationFrame` محدود کنید. همچنین `fillGrid` را یک‌جا با `insertAdjacentHTML` بسازید نه با حلقهٔ `appendGridRow()`.

---

## 🟢 موارد جزئی

| # | مورد | فایل | راهکار |
|---|---|---|---|
| 17 | `validate_backup_connection` طول `name` حساب/گروه را چک نمی‌کند | `db.rs` | `length(name) > 255` را هم به کوئری اضافه کنید |
| 18 | `save_settings` سال مالی را فقط از نظر طول چک می‌کند، نه عدد بودن | `db.rs` | حداقل `chars().all(|c| c.is_numeric() \|\| c == '/' \|\| c == '-')` بگذارید |
| 19 | در `get_settings` مقادیر پیش‌فرض تکراری هاردکد شده | `db.rs` | از `DEFAULT_SETTINGS` استفاده کنید |
| 20 | `report_meta` در هر پیش‌نمایش دوباره تنظیمات را می‌خواند | `lib.rs` | نتیجه را برای چاپ بعدی کش کنید یا در `AppState` نگه دارید |
| 21 | `open_path` پسوند `.htm` را می‌پذیرد ولی در نام فیلد به `.html` محدود می‌شود | `lib.rs` | سازگار کردن |
| 22 | `today_jalali` می‌تواند بین دو بار فراخوانی در نیمه‌شب تفاوت کند | `db.rs` | قابل قبول برای این برنامه |
| 23 | `account-definition.js` گروه را در حالت `edit` قفل می‌کند حتی اگر مجاز باشد | `account-definition.js` | `group.disabled = !allowGroupChange` است ولی در `parentsFor` هم `groupSelect.disabled = !isEdit` در `app.js` وجود دارد — یکسان‌سازی |
| 24 | `.sidebar.is-hidden` در `index.html` از ابتدا فعال است | `index.html` | اگر سایدبار در اولین اجرا مفید است، حذف کنید یا در `localStorage` ذخیره کنید |
| 25 | `generate_handler!` هیچ فرمان `retry_database` یا `report_error` ندارد | `lib.rs` | افزودن برای بهبود recoverability |
| 26 | در `voucher.js`، `Number.parseInt` روی `MAX_VOUCHER_NUMBER` بدون `BigInt` است | `voucher.js` | فعلاً ۲^۵۳ − ۱ از `Number.MAX_SAFE_INTEGER` کوچک‌تر است؛ ولی اگر تغییر کرد، مشکل می‌شود |
| 27 | اعتبارسنجی `validateDraft` تاریخ شمسی کامل انجام نمی‌دهد | `voucher.js` | با `Intl.DateTimeFormat('fa-IR-u-ca-persian')` یا همان `validate_voucher_date` قابل بهبود است |

---

## 💡 پیشنهادات برای کامل‌تر شدن برنامه

### الف) پشتیبان‌گیری
1. **پشتیبان خودکار قبل از migration**: در `initialize`، قبل از `apply_schema_migrations` اگر نسخه قدیمی بود، یک کپی `before-migration-<timestamp>.sqlite3` در پوشهٔ داده بسازید. اگر migration شکست خورد، کاربر می‌تواند برگردد.
2. **پشتیبان خودکار دوره‌ای**: هر ۲۴ ساعت یک نسخه در `backups/` نگه دارید (با pruning مثلاً ۷ نسخهٔ آخر).

### ب) گزارش‌ها
3. **افزودن ستون «شرح سند» به تراز آزمایشی**: الان فقط عنوان حساب و گروه هست؛ افزودن `voucher.description` در drill-down مفید است.
4. **گزارش «گردش حساب» با ستون مانده تجمعی به تفکیک سطح**: الان `get_ledger` برای والد، همهٔ نوادگان را قاطی می‌کند. افزودن گزینهٔ «فقط حساب جاری / شامل زیرحساب‌ها» به UI.
5. **گزارش مقایسه‌ای دو دوره**: منوی «صورت‌های مالی» آن را لیست کرده ولی پیاده‌سازی نشده.

### ج) رابط کاربری
6. **Keymap قابل کشف**: در `voucher.js` کلیدهای میانبر وجود دارد (F2، Ctrl+S/N/P/F، Ctrl+C/V، F4/Space) ولی جایی فهرست نشده. دکمهٔ «میانبرها» را به Help اضافه کنید.
7. **پیام «سند بازیابی‌شده» در `voucher.js`**: اگر پنجرهٔ سند باز باشد و کاربر از منوی اصلی «بازیابی پشتیبان» بزند، `database-restored` ارسال می‌شود و `handleDatabaseRestored` ذخیره را قفل می‌کند. UX خوب است ولی بهتر است دکمهٔ «بستن پنجره» هم هایلایت شود.
8. **پیشرفت برای bulk import**: هنگام `parse_bulk_vouchers` روی فایل ۱۱۴ هزار ردیفی، UI فریز می‌شود. یک progress dialog با Web Worker یا حداقل `setTimeout` بین مراحل اضافه کنید.
9. **دکمهٔ «جستجوی سراسری»**: جستجو در سند فعلی هست، ولی جستجوی سراسری اسناد (از منوی اصلی) فقط با ورود به صفحهٔ اسناد کار می‌کند.

### د) بک‌اند
10. **`Pool` ساده**: به جای `Mutex<Option<Connection>>`، از `r2d2_sqlite` یا حتی دو کانکشن (یکی read-only برای گزارش‌ها، یکی read-write) استفاده کنید تا گزارش‌های سنگین نوشتن را بلاک نکنند.
11. **`PRAGMA integrity_check` در `initialize`**: در حال حاضر فقط در `validate_backup_connection` انجام می‌شود. اگر DB محلی خراب شود، باز کردن بی‌سر و صدا انجام می‌شود و بعداً خطاهای عجیب می‌دهد.
12. **`PRAGMA optimize` در پایان `initialize`** برای بهبود query planner.
13. **گزارش `schema_version` در `get_settings`** تا UI بتواند در صورت لزوم هشدار بدهد.
14. **`VACUUM` اختیاری**: در تنظیمات، دکمهٔ «فشرده‌سازی پایگاه داده» اضافه کنید.

### هـ) امنیت
15. **`withGlobalTauri: true`**: هرچند CSP محدود است، بهتر است به‌تدریج به importهای ES ماژولار Tauri مهاجرت کنید.
16. **`open_path`**: در حال حاضر فقط HTML را می‌پذیرد (خوب است). اگر روزی PDF/تصویر اضافه شد، حتماً extension whitelist را نگه دارید.
17. **محدودیت اندازهٔ فایل اکسل**: `MAX_EXCEL_FILE_BYTES = 32MB` خوب است؛ ولی `calamine` ممکن است هنگام خواندن فایل‌های zip bomb مثل قبل بزرگ حافظه مصرف کند. `ensure_sheet_dimensions` کمک می‌کند، اما `worksheet_range_at(0)` ممکن است خودش range را کامل بارگذاری کند. اگر نگران هستید، `Reader::with_header_row` یا `worksheet_range` با محدودهٔ محدود استفاده کنید.

---

## 📁 فایل‌هایی که برای بررسی دقیق‌تر لازم است

برای اینکه بتوانم موارد باقی‌مانده را دقیق‌تر بررسی کنم، لطفاً این فایل‌ها را ارسال کنید:

| فایل | دلیل |
|---|---|
| `capabilities/*.json` کامل (همان که در `capabilities.json` دیدم کافی است، ولی اگر چند فایل مجزاست بفرستید) | تأیید مجوزهای پنجره‌ها |
| `icons/*` | بررسی اندازه و کیفیت آیکون‌ها (NSIS) |
| `fonts/Vazirmatn-FD-*.ttf` (فقط نام و اندازه) | تأیید وجود فونت‌ها برای embed در PDF/HTML |
| هر فایل `*.rs` دیگری در `src-tauri/src/` که ندیدم (مثلاً `jalali.rs` یا `main.rs` اگر جدا دارید) | بررسی کامل بک‌اند |
| `scripts/check-js.mjs` | بررسی اسکریپت lint |
| نمونهٔ `Book1.xlsx` (اگر می‌خواهید عملکرد bulk import را دقیق بسنجم) | تست عملکرد |

---

## 🧪 تست‌های پیشنهادی برای افزودن

- **`transfer_voucher_lines` با حالت copy**: تست موجود نیست؛ فقط move/copy در تست‌های reorganize پوشش داده شده.
- **`restore_from` وقتی فایل مقصد همان مسیر فعال است**: تست `backup_to` دارد ولی `restore_from` با مسیر یکسان تست نشده (کد خطا می‌دهد ولی تست ندارد).
- **`apply_schema_migrations` وقتی نسخهٔ DB جدیدتر از برنامه است**: تست ندارد.
- **`get_ledger` با حساب والد و `from` خالی**: تست پوشش دارد ولی `from` خالی صریحاً تست نشده.
- **bulk import با فایل خالی/فقط هدر**: تست ندارد.
- **`voucher.js` با سند بزرگ (۵۰۰+ ردیف)**: تست مرورگر لازم است.

---

## خلاصهٔ اولویت‌بندی

| اولویت | مورد | اثر |
|---|---|---|
| 🔴 ۱ | هاردکد `schema_version` | خطر داده در آپدیت آینده |
| 🔴 ۲ | `unchecked_transaction` | نگهداشت‌پذیری |
| 🔴 ۳ | تجمیع در Rust برای `get_trial_balance` | کندی و قفل شدن UI |
| 🔴 ۴ | بدون pagination برای `list_vouchers`/`get_journal` | فریز UI با داده‌های زیاد |
| 🟠 ۵ | قرمز شدن حساب‌های سطح ۴ legacy | UX گمراه‌کننده |
| 🟠 ۶ | `length` ناسازگار JS/Rust | خطای مرزی در متن |
| 🟠 ۷ | نبود `retry_database` | گیر افتادن پس از شکست restore |
| 🟢 ۸ | سایر موارد جدول ۱۷–۲۷ | بهبود جزئی |

اگر مایل باشید، می‌توانم برای هر یک از موارد ۱ تا ۵، **patch دقیق** (کد قبل/بعد) بنویسم تا مستقیماً اعمال کنید.