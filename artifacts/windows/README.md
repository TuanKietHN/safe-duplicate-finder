# Bằng chứng bộ cài Windows

## Online setup 0.2.0 — 2026-08-09

| Trường | Giá trị |
|---|---|
| Tệp phát hành | `target/release/online-installer/safe-dedupe-setup-0.2.0-x64.exe` |
| Kích thước | 3.434.960 byte (khoảng 3,28 MiB; đạt ngân sách dưới 15 MiB) |
| SHA-256 setup | `6C736129B7C9074E093C71022B9F69A65D9E3158FBC3168C552CF951938F5606` |
| Phiên bản sản phẩm | `0.2.0` |
| Authenticode | `NotSigned` |
| Helper native | 567.808 byte; SHA-256 `874B296405A9956E36D7B0A39AED3C88C43DE55D9B353D46445D181B8AFE245D` |
| Ứng dụng release | 9.688.064 byte; SHA-256 `D6B6A129EC7C0FEA23A50D354DDCE32F8FD6F06410F6C10B5BC7D9EDD3668E48` |
| Manifest Runtime | SHA-256 `38E3EC36C1FB20769717F9280771136F37D2605B4F9D95F1F8A2B31EBEB3FBE0` |
| Lệnh dựng | `.\installer\windows\build-online-installer.ps1` |

Manifest ghim Microsoft Edge WebView2 Evergreen Standalone x64 dài 209.605.840 byte, SHA-256
`4AC55375E52435B5AAF2E2E76D81F539A2602CEA38D4B647F5FAADE467C6E078`. Helper dùng WinHTTP,
buffer cố định 64 KiB, tối đa hai worker, `.part`/Range, kiểm tra kích thước và SHA-256 trước khi chạy.
Kiểm toán `dumpbin /dependents` xác nhận helper không phụ thuộc WebView2 hoặc VC++ Redistributable; app
chỉ phụ thuộc DLL hệ thống/UCRT API set và cần WebView2 ở thời điểm chạy.

`release-checksums.json` được tạo cạnh EXE nhưng toàn bộ `target/` chủ đích bị Git bỏ qua. Hãy tải EXE
và file bằng chứng lên GitHub Releases; không commit nhị phân vào kho mã. Bản này đã đạt kiểm tra tĩnh,
118 test Rust, 15 test frontend, `cargo deny` và `npm audit` 0 lỗ hổng. Kiểm thử cài/gỡ trên máy sạch
độc lập và ký nhà phát hành vẫn là cổng phát hành công khai.

## Mốc bộ cài Windows 0.1.9 (lịch sử)

Bản phát triển được tạo ngày 2026-07-23 bằng NSIS bundler của Tauri trong môi trường Visual Studio
2022 Build Tools x64.

| Trường | Giá trị |
|---|---|
| Tệp | `target/release/bundle/nsis/Trình tìm tệp trùng lặp an toàn_0.1.9_x64-setup.exe` |
| Kích thước | 3.310.800 byte |
| SHA-256 | `CCA17BDF473BD7AF7A4A2E4B89DD8618C709BF4495331E54B881A77E1BD89A17` |
| Lệnh dựng | `npm run tauri -- build --bundles nsis` |

Tệp thực thi được chủ đích bỏ qua trong Git. Trước khi nâng cấp, cơ sở dữ liệu SQLite được sao lưu
nhất quán tại `artifacts/user-data-backup/state-before-history-0.1.9.db` và đạt
`integrity_check = ok`. Bản 0.1.9 được cài im lặng đè lên 0.1.8 với mã thoát 0 tại
`%LOCALAPPDATA%/Programs/Trình tìm tệp trùng lặp an toàn`; tệp thực thi và mục đăng ký đều báo
0.1.9.

Migration v6 cùng bốn chỉ mục lịch sử đạt và cơ sở dữ liệu sau nâng cấp vẫn có tính toàn vẹn `ok`.
Lịch sử thật có 230.672 snapshot, 9.667 tệp thuộc 4.129 nhóm trùng; truy vấn đầy đủ 50 dòng trả về
trong khoảng 0,99 giây khi ứng dụng đang chạy. Kế hoạch
`c709deb6-d9de-4922-99cb-d6bfbf4d2d3c` được tạo và khóa cho phiên
`4a1c0760-a788-476b-987b-45d9410ad250`: 1.149 nhóm, 1.801 tệp dự kiến cách ly và
9.295.338.387 byte. Chỉ kế hoạch cơ sở dữ liệu được tạo; chưa áp dụng cách ly hay xóa tệp. Ứng dụng
0.1.9 tự khôi phục mã dự án/phiên/kế hoạch vào trạng thái `WebView`, được mở lại và phản hồi bình
thường.

`Get-AuthenticodeSignature` báo `NotSigned`, nên đây vẫn là bộ cài phát triển. Kiểm thử trên máy sạch
độc lập và ký nhà phát hành vẫn là cổng phát hành bắt buộc.
