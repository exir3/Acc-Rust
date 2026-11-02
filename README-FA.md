# بسته اصلاحات بررسی فنی Acc-Rust

این بسته شامل یک **پچ قابل اعمال روی سورس موجود** است؛ کل مخزن GitHub را شامل نمی‌شود. پچ برای شاخه فعلی `main` مخزن خصوصی `exir3/Acc-Rust` و بر اساس نسخه‌ای که هنگام بازبینی در ۲ نوامبر ۲۰۲۶ خوانده شد تهیه شده است.

## روش اعمال

۱. از پایگاه داده برنامه (`hesabdari.sqlite3`) نسخه پشتیبان تهیه کنید و از پوشه پروژه نیز یک کپی جداگانه داشته باشید.

۲. فایل `Acc-Rust-review.patch` را در ریشه پروژه Rust قرار دهید؛ همان پوشه‌ای که پوشه‌های `src-tauri` و `ui` در آن هستند.

۳. در PowerShell یا CMD از ریشه پروژه اجرا کنید:

```powershell
git apply --check .\Acc-Rust-review.patch
if ($LASTEXITCODE -ne 0) { throw 'Patch check failed' }
git apply .\Acc-Rust-review.patch
cargo fmt --manifest-path .\src-tauri\Cargo.toml
cargo test --manifest-path .\src-tauri\Cargo.toml
cargo check --manifest-path .\src-tauri\Cargo.toml
```

اگر `git apply --check` موفق نشد، پچ را به زور اعمال نکنید؛ احتمالاً فایل‌های مخزن از زمان بررسی تغییر کرده‌اند. در این حالت فایل‌های فعلی را با نسخه زمان بازبینی مقایسه کنید.

۴. پس از موفقیت آزمون‌ها، نسخه دسکتاپ را با اسکریپت پروژه بسازید:

```powershell
.\build.bat portable
```

ابتدا روی یک کپی از پایگاه داده آزمایش کنید؛ سپس پس از تأیید عملی، جایگزینی نسخه اصلی را انجام دهید.

## فایل‌های تغییرکرده

- `src-tauri/src/db.rs`
- `src-tauri/src/vouchers.rs`
- `ui/voucher.js`

هیچ تغییری مستقیماً روی GitHub ثبت نشده است؛ این بسته برای بررسی و آزمایش محلی شماست.

## دامنه آزمون

در این محیط امکان اجرای `cargo fmt`، `cargo test`، `cargo check` یا ساخت نسخه Windows وجود نداشت، چون Rust/Cargo در محیط اجرایی نصب نبود. همچنین به‌دلیل محدودیت شبکه نتوانستم مخزن خصوصی را clone کنم. پچ بر مبنای محتوای فایل‌های دریافت‌شده از GitHub تهیه شده و پیش از استفاده باید در checkout محلی شما با `git apply --check` و سپس آزمون‌های بالا اعتبارسنجی شود.
