# Hướng dẫn xác thực nhanh

## Điều kiện cần

- Windows 10/11 x86_64
- Microsoft C++ Build Tools với Desktop development with C++
- WebView2 Runtime
- Rust stable MSVC 1.97.1 hoặc bản stable mới hơn tương thích
- Node.js 24.x và npm 11.x

## Biên dịch và kiểm thử

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
npm --prefix apps/desktop ci
npm --prefix apps/desktop run test
npm --prefix apps/desktop run build
cargo build --workspace --release
```

Ứng dụng release không nhập `VCRUNTIME140.dll`/`MSVCP140.dll`; không cần tải Visual C++
Redistributable cho máy đích Windows 10/11. WebView2 là Runtime không thuộc hệ thống duy nhất.

## Dữ liệu an toàn đầu-cuối

Script CLI xác định có tại:

```powershell
.\tests\quickstart-smoke.ps1
```

1. Tạo dữ liệu tạm ngoài thư mục cơ sở dữ liệu và cách ly.
2. Thêm bản sao độc lập giống nhau, cặp cùng kích thước khác một byte, cặp hard link, đường dẫn
   Unicode, đường dẫn dài, tệp bị khóa và tệp bị sửa trong khi băm.
3. Tạo dự án; thêm cả thư mục cha và thư mục lồng nhau. Xác nhận chồng lấn được báo.
4. Chạy quét nghiêm ngặt. Chỉ tệp ổn định, độc lập, trùng từng byte và cùng tên được vào nhóm.
5. Chạy thử. Ghi tệp giữ lại được đề xuất và xác nhận không đường dẫn dữ liệu nào đổi.
6. Tiêm crash sau đổi tên nhưng trước cập nhật cơ sở dữ liệu. Khởi động lại và kiểm tra phục hồi.
7. Đối soát, xác minh đích cách ly rồi khôi phục về đường dẫn gốc.
8. So sánh bằng chứng nội dung đầy đủ và nhật ký kiểm toán/giao dịch.

## Bất biến mong đợi

- Tệp cùng kích thước nhưng khác một byte không bao giờ là tệp trùng.
- Tệp đã thay đổi là không ổn định và không bao giờ bị di chuyển.
- Mỗi nhóm luôn còn ít nhất một bản sao đã xác minh.
- Số byte thu hồi chỉ tăng sau khi xác minh đích.
- Khôi phục trả đúng nội dung về đường dẫn gốc mà không ghi đè.
- Chạy lại dự án không gây thao tác trùng lặp.
- Không lệnh mặc định/nguồn nào cho phép xóa vĩnh viễn; chỉ quy trình cách ly hết thời hạn, chọn riêng
  từng UUID trong registry mới được chấp nhận.

## Hồ sơ xác thực cục bộ

Ngày 2026-07-22, CLI phát hành đã mở và kiểm tra toàn vẹn cơ sở dữ liệu tạm mới, tạo dự án/thư mục gốc
nghiêm ngặt, quét chỉ đọc 24 tệp với 0 lỗi và đọc kết quả nhóm đã chứng minh rỗng. Quickstart xác định
hiện tại còn chứng minh quét → kế hoạch → chạy thử → cách ly đã xác minh → khôi phục đã xác minh cho
một nhóm trùng (1 tệp / 28 byte), nội dung khôi phục khớp.

Mốc tự động có 88 test Rust và 12 test frontend đạt. Nó thử từ chối khác một byte, hai băm đầy đủ,
đường dẫn Unicode dài, tệp sparse logic >4 GiB, bí danh hard link bên ngoài, từ chối nguồn thay đổi/bị
thay, đổi tên không ghi đè gắn handle, độ bền manifest-trước-SQLite, sáu ranh giới cách ly được tiêm
lỗi, phục hồi đích hỏng, cách ly/khôi phục đã xác minh và phục hồi xóa chỉ trong cách ly. Tiêm lỗi cấp
thiết bị, Docker runtime, xác thực bộ cài trên máy sạch và lượt 1 TB vẫn là qualification phát hành.

## Đóng gói Windows

```powershell
.\installer\windows\build-online-installer.ps1
```

Script thực hiện theo thứ tự có kiểm tra:

1. Chạy kiểm thử downloader/manifest/progress và kiểm tra định dạng.
2. Build helper Runtime native với CRT tĩnh, LTO, một codegen unit và strip symbol.
3. Build frontend + một Tauri NSIS setup, đặt `webviewInstallMode=skip` và nhúng helper vào resource tạm.
4. Xác minh kích thước/SHA-256 của setup, helper và manifest Runtime.
5. Ghi EXE online cuối cùng cùng bằng chứng SHA-256 vào `target/release/online-installer/`.

## Xác thực tiến độ Runtime bằng byte thật

```powershell
cargo test -p safe-dedupe-runtime-installer
.\installer\windows\verify-installer.ps1 -InstallerPath `
  .\target\release\online-installer\safe-dedupe-setup-0.2.0-x64.exe
```

Các fixture phải chứng minh: tải mới, resume 206, server bỏ qua Range trả 200, sai độ dài, sai SHA-256,
cache hoàn chỉnh, hủy/retry, hai tệp đồng thời và giới hạn bộ nhớ. Giá trị UI được đối chiếu với số byte
fixture server thực nhận; không có bộ đếm thời gian giả.

## Xác thực gỡ cài đặt

Sau khi cài trên tài khoản Windows sạch:

1. Kiểm tra shortcut ứng dụng và shortcut `Gỡ cài đặt ...` trong Start menu.
2. Tạo dữ liệu mồi trong `%APPDATA%\io.github.safeduplicate.finder` và
   `%LOCALAPPDATA%\io.github.safeduplicate.finder`.
3. Tạo tệp mồi trong `.safe-duplicate-finder-quarantine` trên ổ nguồn thử nghiệm.
4. Chạy shortcut gỡ cài đặt và xác nhận thư mục cài đặt, hai product data root, cache installer,
   registry và shortcut đã biến mất.
5. Xác nhận tệp cách ly và manifest phục hồi vẫn còn, hash không đổi.

MSI là tùy chọn, cần tính năng VBSCRIPT của Windows và điều kiện WiX. Bằng chứng phát hành phải gồm
máy sạch không có WebView2, máy đã có WebView2, resume/retry/tamper, cài/khởi chạy, dữ liệu quét-only,
upgrade giữ dữ liệu, gỡ cài đặt xóa app-local data và giữ nguyên dữ liệu cách ly.
