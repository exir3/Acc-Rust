بررسی روی شاخهٔ `main` در commit [`c9f0143`](https://github.com/exir3/Acc-Rust/commit/c9f01431729b41c8f2140f0af3e5ceadf7aa56bb) (۸ نوامبر ۲۰۲۶، «Retry NSIS bundling when the GitHub download fails») انجام شد. [`Docs/Project-Status.md`](https://github.com/exir3/Acc-Rust/blob/main/Docs/Project-Status.md) هنوز مبنای `cb5d694` را نوشته و نسبت به این HEAD عقب است. متن زیر را می‌توان عیناً به ایجنت VS Code داد.

## دستور کار

مخزن محلی `Acc-Rust` (Tauri 2، Rust، SQLite، UI در `ui/`) را اصلاح کن. بازنویسی نکن. رفتار حسابداری فعلی را حفظ کن مگر جایی که پایین صریحاً گفته شده تغییر کند. بعد از هر مورد: تست مرتبط را اضافه یا به‌روز کن، `cargo fmt`، `cargo clippy --all-targets -- -D warnings`، `cargo test`، `npm run check:js`، `npm run lint:js` و `npm test` را پاس کن. پیام commit مبهمِ `commit` ننویس.

### دست نزن

- مبالغ `i64` ریال و `BigInt` در `ui/js/voucher/amounts.js` و سقف `src-tauri/src/limits.rs`. به `f64` برنگردان.
- تریگر یک‌طرفه بودن ردیف در `src-tauri/src/db/schema.rs` (`voucher_lines_valid_insert/update`): فقط یکی از بدهکار یا بستانکار مثبت است.
- تریگر سند دائمی، جفت حساب پایه/تفصیلی، و `expected_updated_at`.
- `PRAGMA synchronous` پیش‌فرض (FULL) و مسدود بودن `HESABDARI_SMOKE_DATA_DIR` در release داخل `src-tauri/src/db/connection.rs`.
- تاریخچهٔ `account_classification_history` (schema v13). برای دورهٔ قبل از ارتقا، تاریخ جعلی نساز.
- `ensure_printable` موجود در `src-tauri/src/commands/print.rs` را ضعیف نکن.
- تا `Docs/Accounting-Report-Signoff.md` مقدار `Status: approved` نگیرد، gate انتشار را برندار.

معماری فعلی (WebView فقط `invoke`، قاعده در Rust و SQLite) درست است.

## اصلاح‌های کد

### ۱. HTML ذخیره‌شدهٔ گزارش را از کلاینت نپذیر

**شدت:** بالا (امنیت محلی)  
**شاهد:** `save_report_html` در `src-tauri/src/commands/print.rs` رشتهٔ `html` را از WebView روی دیسک می‌نویسد و مسیر canonical را در `AppState.report_files` می‌گذارد. `get_print_doc` همان سند را از `print_docs` حذف می‌کند، پس سرور دیگر نسخهٔ اصلی را ندارد. `open_path` فقط همین allowlist را باز می‌کند. پنجرهٔ پیش‌نمایش با `withGlobalTauri: true` در `src-tauri/tauri.conf.json` به `invoke` دسترسی دارد.

**کار:** سند `PrintDoc` را تا بسته شدن پنجرهٔ `print-preview-*` در state نگه دار (حذف در `get_print_doc` فقط یک کپی برای رندر بدهد). `save_report_html` پارامتر `html` را نادیده بگیرد و فقط HTML ذخیره‌شدهٔ همان `window.label()` را بنویسد. نام فایل را به basename بدون `..`، اسلش و کاراکتر کنترلی محدود کن و پسوند را فقط `.html` بگذار. تست: فراخوانی با HTML غیر از سند سرور نباید محتوای دلخواه بنویسد.

### ۲. گیت چاپ رسمی را به تراز آزمایشی برسان

**شدت:** متوسط (کنترل حسابداری)  
**شاهد:** `ensure_printable` فقط برای سود و زیان، ترازنامه، تغییرات حقوق مالکانه، بهای تمام‌شده، تخصیص سود و جریان نقد صدا زده می‌شود. `open_trial_balance_preview`، `open_journal_preview` و `open_ledger_preview` حتی با `scope=official` بدون Integrity چاپ می‌کنند. دفتر روزنامه و دفتر کل «صورت مالی رسمی» نیستند؛ تراز آزمایشی هست و خطای `trial_balance` در `src-tauri/src/vouchers/reports/integrity.rs` همان عدم‌تراز است.

**کار:** برای پیش‌نمایش رسمی تراز آزمایشی، `run_integrity_checks` را با همان بازه و `ReportScope::Official` اجرا کن و `ensure_printable` را اعمال کن. حالت کاری را مسدود نکن. دفتر روزنامه، دفتر کل و چاپ یک سند را مسدود نکن؛ در HTML آن‌ها اگر Integrity خطا دارد یک نوار هشدار (با `html()` موجود در `src-tauri/src/report/layout.rs`) نشان بده. تست Rust: تراز نامیزان، چاپ رسمی تراز را رد کند و چاپ کاری را رد نکند.

### ۳. دورهٔ صورت جریان نقد در چاپ دیده نمی‌شود

**شدت:** متوسط (نمایش حسابداری، نه فرمول)  
**شاهد:** در `supplementary_statements.rs` گردش دوره از `opening_date` تا `as_of` با `statement_pairs` محاسبه می‌شود، اما `cash_flow_print_doc` در `src-tauri/src/report/cash_flow.rs` فقط `تا تاریخ {to_date}` می‌گذارد. صورت غیرمستقیم دوره‌ای است؛ عنوان «تا تاریخ» آن را انباشته از ابتدای فعالیت نشان می‌دهد.

**کار:** تاریخ شروع واقعیِ همان محاسبه (تاریخ سند افتتاحیهٔ مبنای دوره، نه امروز) را روی `CashFlowReport` بگذار و در `head` به‌صورت `از {from} تا {to}` چاپ کن. اگر سند افتتاحیه نیست، همان جملهٔ محدودیت فعلی را نگه دار و تاریخ ساختگی نگذار. محاسبهٔ `opening_cash` / `net_change` / `variance` را عوض نکن. یک assert در تست موجود `vouchers/tests.rs` (مورد `opening_cash == 400_000`) اضافه کن که عنوان چاپ هر دو تاریخ را داشته باشد.

### ۴. سقف خاموشِ `skip_types` در جمع گردش

**شدت:** پایین، ولی خطای بی‌صدا  
**شاهد:** `account_activities` در `src-tauri/src/vouchers/accounting.rs` فقط سه نوع را به `?3/?4/?7` وصل می‌کند. نوع چهارم دور ریخته می‌شود و هیچ خطا نمی‌دهد. امروز فراخوان‌ها `[]` یا کمتر از چهار نوع می‌فرستند.

**کار:** لیست را با placeholder به تعداد واقعی بساز. اگر خالی است شرط اضافه نکن. تست: چهار نوعِ skip واقعاً از جمع خارج شوند و نوع پنجمِ خارج از لیست بماند.

### ۵. فرار HTML در عنوان دوره، یک‌لایه و در محل درج

**شدت:** پایین (سخت‌سازی)  
**شاهد:** `head()` در `layout.rs` مقدار `period_text` را خام داخل `<b>` می‌گذارد. الان همه‌ٔ فراخوان‌ها از قبل `date()`/`html()` شده‌اند، پس دو بار escape می‌شود اگر همین‌جا هم escape کنی.

**کار:** `period()` و `date()` متن خام برگردانند و تنها `head()`/`html()` اسکیپ کند. تست واحد: نام شرکت و تاریخ حاوی `<` و `"` در HTML سند چاپ، entity شوند و یک‌بار.

### ۶. ممیزی بدون عامل

**شدت:** متوسط برای استفادهٔ چندنفره؛ برای تک‌کاربره محدودیت شناخته‌شده  
**شاهد:** `src-tauri/src/db/audit.rs` فقط `action`, `entity_type`, `entity_id`, `before_value`, `after_value`, `details`, `app_version` دارد. `changed_by` در تاریخچهٔ طبقه‌بندی هست، در audit سراسری نیست.

**کار:** مهاجرت افزودنی (نه بازنویسی جدول): ستون‌های nullable `actor` و `workstation`. در API فعلی یک مقدار ثابت محلی مثل نام رایانه از محیط سیستم بنویس؛ کاربر ساختگی نساز. لاگ‌های قدیمی `NULL` بمانند. این مورد را پیش از بقیه انجام نده.

## انجام نده (مگر اندازه‌گیری جدا)

- Connection pool روی Mutex سراسری `Database::with_conn`، تا benchmark ۱۰۰هزار و ۳۰۰هزار ردیف ثبت نشود.
- Virtual scrolling تراز آزمایشی (`ui/js/pages/trial-balance.js` کل درخت را می‌گیرد) را با تغییر قرارداد جمع‌ها قاطی نکن.
- شکستن `ui/js/pages/settings.js` (حدود ۶۸هزار بایت) و `ui/account-definition.js` فقط در یک PR رفتاریِ صفر، بعد از موارد ۱ تا ۴.
- SQLCipher روی دیتابیس روشن. سیاست فعلی: فایل فعال plaintext، پشتیبان `.hacc` با AES-256-GCM. در `Docs/Data-Protection-and-Release-Policy.md` بماند.
- هم‌تراز کردن عددی با WebApp و امضای حسابدار. آن‌ها کار انسان است، نه تولید SHA در `Accounting-Report-Signoff.md`.

## پذیرش

1. ذخیرهٔ HTML گزارش فقط بایت‌های ساخته‌شده در Rust را می‌نویسد.
2. چاپ رسمی تراز آزمایشی با خطای Integrity بسته است؛ چاپ کاری باز است.
3. عنوان جریان نقد، شروع و پایان همان بازه‌ای را نشان می‌دهد که موتور جمع می‌زند.
4. بیش از سه نوعِ حذف‌شده از گردش، بدون اثر خاموش نمی‌مانند.
5. `Docs/Project-Status.md` به SHA همین commit به‌روز شود و این چهار مورد از یافتهٔ باز خارج شوند؛ P1-02 و P1-03 (تأیید حسابدار و fixture چهار صورت تکمیلی) باز بمانند.

ترتیب اجرا: ۱، ۲، ۳، ۴، ۵. مورد ۶ جدا و اختیاری است.