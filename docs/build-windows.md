# Biên dịch và đóng gói trên Windows

## Điều kiện cần

- Windows 10/11 x86_64. WebView2 chỉ cần để chạy app/dev; helper trong bộ cài không phụ thuộc WebView2.
- Visual Studio 2022 Build Tools: Desktop development with C++ và Windows SDK.
- Rust stable MSVC 1.97.1 (`rustup toolchain install 1.97.1-x86_64-pc-windows-msvc`).
- Node.js 24.18.0 LTS và npm 11.x.

## Kiểm tra

Mở Developer PowerShell for VS 2022 rồi chạy từ thư mục gốc kho mã:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
npm --prefix apps/desktop ci
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run test
npm --prefix apps/desktop run build
cargo build --release -p safe-dedupe
```

## Phát triển

```powershell
npm --prefix apps/desktop run tauri dev
```

Tauri lưu trạng thái desktop trong thư mục dữ liệu cục bộ của ứng dụng. Khi dùng CLI trong thực tế,
phải chỉ định đường dẫn cơ sở dữ liệu rõ ràng và đặt nó ngoài các thư mục nguồn.

## Bộ cài online NSIS

```powershell
.\installer\windows\build-online-installer.ps1
```

Script build helper native với CRT tĩnh, sau đó nhúng helper vào một Tauri NSIS setup dùng
`webviewInstallMode=skip`. Hook NSIS chỉ báo hoàn tất sau khi helper đã phát hiện hoặc cài và phát hiện
lại WebView2. Đầu ra phát hành nằm trong `target/release/online-installer/`, kèm
`release-checksums.json` cho setup, helper, manifest và app release.

`target/` chỉ là cache và đầu ra build cục bộ, không được đưa vào Git hay bộ cài. Trước khi chạy
`cargo clean`, hãy tải lên GitHub Releases hoặc sao chép ra ngoài `target/` hai tệp
`safe-dedupe-setup-<phiên-bản>-x64.exe` và `release-checksums.json`.

Helper chỉ tải artifact Microsoft được ghim trong `installer/runtime-manifest.json`; cache và log cài
đặt nằm dưới `%LOCALAPPDATA%\io.github.safeduplicate.finder\installer-cache`. Cache hoàn chỉnh phải
đúng cả kích thước và SHA-256; `.part` hợp lệ được tiếp tục bằng HTTP Range.

Kiểm toán import của `safe-dedupe-desktop.exe` cho bản 0.2.1 chỉ thấy DLL hệ thống Windows/UCRT API
set, không có `VCRUNTIME140.dll` hoặc `MSVCP140.dll`; runtime không thuộc hệ thống duy nhất cần xử lý là
WebView2. Helper được build với `-C target-feature=+crt-static` để không tạo thêm vòng phụ thuộc VC++
Redistributable.

Hãy kiểm tra trên tài khoản người dùng chuẩn sạch: cài đặt khi chưa có WebView2, cài đặt khi WebView2
đã hợp lệ, ngắt/tải tiếp, khởi chạy, tạo dự án, quét dữ liệu dùng thử, đóng/mở lại, khôi phục một mục
cách ly, gỡ cài đặt và xác nhận dữ liệu app bị xóa nhưng tài liệu trong vùng cách ly không đổi.

Gỡ cài đặt rõ ràng xóa hai root cố định `%APPDATA%\io.github.safeduplicate.finder` và
`%LOCALAPPDATA%\io.github.safeduplicate.finder`, registry/shortcut sản phẩm và thư mục cài đặt. Nó
không dò ổ đĩa và không xóa `.safe-duplicate-finder-quarantine`, thư mục nguồn hoặc nơi xuất báo cáo.

MSI là tùy chọn và cần WiX/VBScript. Không ký hoặc công bố bộ cài khi chưa có danh tính ký mã được tổ
chức phê duyệt cùng bằng chứng phát hành trong `release-evidence.md`.
