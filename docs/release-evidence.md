# Bằng chứng phát hành

## Bằng chứng phát triển cục bộ 0.1.9 — 2026-07-23

- [x] Có đầy đủ hiến pháp, đặc tả, kế hoạch, tác vụ, hướng dẫn nhanh và hợp đồng Spec Kit.
- [x] `cargo fmt --all -- --check` đạt.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` đạt.
- [x] `cargo test --workspace --all-features` đạt đủ 90 test Rust, 0 lỗi. Phạm vi gồm điều khiển quét
  và backpressure, hai băm đầy đủ, chống thay đường dẫn, hard link, ma trận lỗi cách ly/khôi phục, xác
  nhận và phục hồi gián đoạn khi xóa vĩnh viễn, tương thích CLI/core, độ bền SQLite và xác thực lệnh
  Tauri.
- [x] Build production frontend, ESLint, Prettier và toàn bộ 15 test Vitest đạt.
- [x] `npm audit` báo 0 lỗ hổng đã biết.
- [x] `cargo audit` không báo lỗi lỗ hổng. 17 cảnh báo được chấp nhận là crate bắc cầu không còn bảo
  trì/unsound, chủ yếu là phụ thuộc GTK3/Linux không liên kết vào đích Windows.
- [x] `cargo deny check -A warnings` đạt advisories, bans, licenses và sources; cảnh báo phiên bản lặp
  và bảo trì vẫn được hiển thị.
- [x] Đánh giá mã nguồn ngoại tuyến/riêng tư không thấy HTTP client runtime, API từ xa, phân tích,
  trình kiểm tra cập nhật hoặc bộ xuất telemetry. CSP Tauri chỉ cho self/IPC/tài nguyên cục bộ; URL
  phát triển là localhost. Nhật ký khởi động CLI không tuần tự hóa đối số lệnh hoặc token xóa.
- [x] Kiểm tra trực quan toàn bộ sáu màn hình tiếng Việt ở 1280×820 không thấy tràn ngang, lỗi bảng
  điều khiển, `NaN` hoặc `undefined`; vùng xóa vĩnh viễn được tách rõ và vô hiệu khi chưa chọn mục.
- [x] Bản 0.1.1 tự tải trang kết quả sau khi chuyển từ phiên quét hoàn tất. Kiểm tra quy mô thực tế
  527 nhóm/1.087 tệp hiển thị đúng 560 bản sao có thể cách ly và 13,5 GiB có thể thu hồi, không tràn
  ngang; cơ sở dữ liệu người dùng xác nhận đủ 527 nhóm bền vững.
- [x] Bản 0.1.2 chuyển các lệnh Tauri đọc/ghi nặng sang nhóm luồng chặn nền và phân trang kết quả 40
  nhóm/lần. Kiểm tra trực tiếp trên dữ liệu 527 nhóm giữ trạng thái phản hồi ở cả 40/40 mẫu trong 10
  giây khi tải, CPU tiến trình chỉ tăng 0,016 giây, bộ nhớ tối đa 46,3 MiB; chuyển sang trang 2/14
  tiếp tục phản hồi ở toàn bộ 20/20 mẫu và không làm tăng CPU có thể đo được.
- [x] Bản 0.1.3 truyền đích đổi tên gắn handle bằng không gian tên Win32 đường dẫn dài. Kiểm thử hồi
  quy trên Windows xác nhận đích cách ly vượt giới hạn MAX_PATH 260 ký tự vẫn được di chuyển không
  ghi đè, giữ nguyên nội dung và danh tính vật lý.
- [x] Bản 0.1.4 thêm chế độ `Xóa ngay` phải bật riêng. Chế độ này bỏ qua duy nhất thời gian giữ 30
  ngày; vẫn bắt buộc chọn từng mục, token 10 phút, câu xác nhận riêng, kiểm tra lại danh tính/kích
  thước/hai băm, ghi ý định bền vững và xóa gắn handle. Kiểm thử xác nhận sai câu không xóa và tệp
  ngoài lựa chọn vẫn còn nguyên.
- [x] Bản 0.1.5 thêm `Chọn tất cả … mục đủ điều kiện` cho kết quả cách ly đang hiển thị và
  `Bỏ chọn tất cả`. Lựa chọn hàng loạt chỉ tạo danh sách UUID; không tự xóa và vẫn đi qua token,
  câu xác nhận chứa đúng số lượng/dung lượng cùng toàn bộ kiểm tra backend.
- [x] Bản 0.1.6 thay hai ô sao chép token/câu dài bằng một ô xác nhận cuối cùng. Giao diện tự gửi lại
  token và câu kỹ thuật; backend vẫn ràng buộc chúng với UUID, số lượng, dung lượng, chế độ và hạn 10
  phút trước khi kiểm tra lại toàn bộ tệp.
- [x] Bản 0.1.7 tách khóa lưu trữ chính xác khi hai tên tệp Unicode khác nhau cùng chuẩn hóa về một
  khóa so sánh; bản lặp hợp lệ không còn làm rollback lô metadata 256 mục. Lỗi chặn được lưu bền
  vững và hiển thị trên giao diện thay cho trạng thái chung chung.
- [x] Truy vấn ánh xạ thành viên nhóm nay ràng buộc `project_id` để dùng chỉ mục tổng hợp
  `(project_id, path_key)` và `(session_id, file_entry_id)`, loại bỏ quét gần O(n²). Phép xác nhận
  trên bản sao phiên thực tế hoàn tất 51.597/51.597 tệp trong 99,9 giây, 0 lỗi, 0 tệp không ổn định,
  1.149 nhóm trùng và 9.295.338.387 byte có thể thu hồi.
- [x] Bản 0.1.8 dùng chung bộ phân giải snapshot có xử lý va chạm Unicode khi tạo mục kế hoạch và
  dùng đủ hai chỉ mục tổng hợp, nên thao tác khóa kế hoạch không còn rollback hoặc chạy gần O(n²).
  Giao diện hiển thị trạng thái đang khóa/lỗi ngay cạnh nút, tự tìm lại kế hoạch đã khóa theo phiên
  và lưu mã dự án/phiên/kế hoạch qua lần khởi động lại `WebView`. Khi bộ nhớ giao diện trống sau
  nâng cấp, backend tự khôi phục ngữ cảnh của kế hoạch khóa mới nhất.
- [x] Bản 0.1.9 thêm lịch sử có phân trang dựng trực tiếp từ snapshot/nhóm trùng/kế hoạch/giao dịch,
  hiển thị mọi vị trí đã chứng minh cùng trạng thái cách ly/xóa mà không nhân đôi nội dung log. Trang
  bảo trì đo riêng SQLite/WAL, manifest, log và `WebView2`; tối ưu DB giữ nguyên mọi hàng lịch sử,
  dọn log chỉ áp dụng tệp chẩn đoán cũ và dọn cache giao diện phải được xác nhận riêng. Migration v6
  bổ sung các chỉ mục truy vấn lịch sử.
- [x] Kiểm thử nhanh theo hướng dẫn Windows hoàn tất tạo dự án/thư mục, quét hai tệp giống nhau thành một nhóm đã
  chứng minh, khóa và chạy thử kế hoạch một tệp/28 byte, cách ly có xác minh, khôi phục không ghi đè và
  kiểm tra nội dung bằng nhau.
- [x] Thao tác Windows xác minh danh tính/token từ handle mở không theo liên kết; thay đường dẫn không
  chạm tệp thay thế. Ràng buộc không ghi đè và cùng ổ đĩa thất bại đóng.
- [x] Append+fsync manifest đứng trước giao dịch trạng thái/kiểm toán SQLite FULL tương ứng. Ma trận
  lỗi cách ly và khôi phục xác định giữ một bản sao tại mọi ranh giới bền vững.
- [x] Xóa vĩnh viễn chỉ khả dụng cho UUID cách ly đã xác minh và được chọn rõ ràng. Chế độ thường yêu
  cầu hết thời hạn; chế độ `Xóa ngay` phải bật riêng và chỉ bỏ qua cổng thời gian. Cả hai dùng token
  một lần ngắn hạn, câu nhập chính xác bằng tiếng Việt, tóm tắt số lượng/byte, giá trị băm
  lựa chọn, kiểm tra trước BLAKE3+SHA-256 đầy đủ của mọi mục, ý định từng mục bền vững và đối soát phục
  hồi. Đường dẫn nguồn, mục không đủ điều kiện, tự dọn, xác nhận sai/cũ và danh tính thay đổi đều bị từ chối. Xem
  `docs/permanent-delete-gate.md`.
- [x] Test shell entrypoint không cần daemon đạt và từ chối lệnh xóa vĩnh viễn. Test portable-delete
  Linux liên kết tĩnh đạt, xác nhận bộ điều hợp ngoài Windows thất bại đóng. Test portable-move
  cross-compile cho `x86_64-unknown-linux-musl`; không tuyên bố lần chạy runtime cuối vì WSL trước tiên
  không tạo được distro, sau đó mount đĩa chỉ đọc và từ chối chạy binary.
- [x] Bộ cài phát triển NSIS x64 cuối được dựng từ mã đã nghiệm thu. Nâng cấp im lặng 0.1.8 → 0.1.9
  đạt với mã thoát 0; tệp thực thi báo 0.1.9. Migration v6 và `integrity_check` đạt. Trên dữ liệu thật,
  lịch sử có 230.672 snapshot/9.667 tệp trùng/4.129 nhóm và trang 50 dòng trả trong khoảng 0,99 giây.
  Kế hoạch `c709deb6-d9de-4922-99cb-d6bfbf4d2d3c` vẫn khóa, không phát sinh giao dịch ngoài ý muốn;
  ứng dụng mở lại có phản hồi. Checksum nằm trong `artifacts/windows/README.md`.

## Bằng chứng phát hành 0.2.1 — 2026-08-09

- [x] Spec Kit đã cập nhật đặc tả, kế hoạch, mô hình dữ liệu, nghiên cứu, quickstart, hợp đồng Runtime
  và tác vụ cho kiến trúc một NSIS setup nhúng helper native.
- [x] `cargo fmt --all -- --check` và
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` đạt.
- [x] `cargo test --workspace --all-targets --all-features` đạt đủ 120 test Rust, 0 lỗi; trong đó 30
  test riêng của runtime installer kiểm tra manifest, byte thật/tốc độ/ETA, cache, preflight, 64 KiB,
  tối đa hai worker, fresh/resume/Range-200/retry, cắt kết nối, hủy, redirect downgrade, SHA-256, exit
  code và hợp đồng NSIS/uninstaller.
- [x] ESLint, Prettier, TypeScript/Vite build và 15 test Vitest đạt.
- [x] `cargo deny check -A warnings` đạt advisories/bans/licenses/sources; `npm audit --audit-level=high`
  báo 0 lỗ hổng sau khi cập nhật các dependency gián tiếp được vá.
- [x] Rust giữ ở stable 1.97.1; CI dùng Node.js 24.18.0 LTS, `actions/checkout@v6` và
  `actions/setup-node@v6`. Frontend đã nâng lên Vite 8.2.1, Vitest 4.1.10, ESLint 10.8.1 và
  TypeScript 6.0.3; TypeScript 7 chưa được dùng vì `typescript-eslint` 8.66.0 chưa hỗ trợ.
- [x] `rusqlite` đã nâng lên 0.40.2, `sha2` lên 0.11.0 và lockfile đã cập nhật toàn bộ gói tương
  thích với Rust 1.97.1.
- [x] `dumpbin /dependents` xác nhận app không nhập `VCRUNTIME140.dll`/`MSVCP140.dll`; helper CRT tĩnh
  chỉ nhập DLL Windows cần thiết như `winhttp.dll`, `comctl32.dll`, `user32.dll` và `advapi32.dll`.
- [x] Online setup 0.2.1 build thành công, ProductVersion đúng, kích thước và SHA-256 được ghi trong
  `artifacts/windows/README.md` cùng `release-checksums.json`. Hash helper,
  manifest và app nằm trong `artifacts/windows/README.md` và `release-checksums.json`.
- [x] Kiểm thử fixture xác nhận tệp cache hoàn chỉnh hợp lệ không mở kết nối, `.part` được tiếp tục,
  response 200 khi resume chỉ khởi động lại mục đó, retry giữ phần đã nhận, dữ liệu sai/cắt ngắn không
  được promote hoặc chạy, HTTPS→HTTP bị từ chối và concurrency không vượt hai.
- [x] Kiểm thử tĩnh hook NSIS xác nhận helper lỗi làm setup `Abort`, shortcut gỡ cài đặt được tạo, cập
  nhật giữ dữ liệu app, còn gỡ rõ ràng chỉ xóa hai root sản phẩm cố định và không có lệnh xóa vùng
  `.safe-duplicate-finder-quarantine`.
- [x] Docker build sạch bằng `rust:1.97.1-trixie` và `debian:trixie-slim` đạt. Smoke test chạy bằng
  user không đặc quyền với `/scan` gắn chỉ đọc; lệnh `check` xác nhận SQLite đạt.

## Các bước thẩm định bên ngoài còn mở trước khi phát hành công khai

- [ ] Cài/khởi chạy trên máy Windows sạch khi chưa có WebView2 và khi WebView2 đã hợp lệ; quan sát byte,
  tổng dung lượng, tốc độ, ETA, trạng thái tệp và tiến độ chung từ mạng thật.
- [ ] Ngắt tiến trình/mạng giữa lần tải Runtime thật, chạy lại để chứng minh resume/no-redownload và
  thử cache sai SHA-256 trên VM có snapshot.
- [ ] Thử nâng cấp giữ dữ liệu và gỡ cài đặt rõ ràng trên VM; xác nhận các root app biến mất nhưng hash
  mọi tài liệu/manifest đã gieo trong vùng cách ly không đổi.
- [ ] Benchmark tham chiếu đầy đủ lớp 1 TB trên phần cứng đích, gồm HDD/SATA SSD/NVMe và nhiều ổ.
- [ ] Ma trận DB đầy/bận/commit, ổ đầy, từ chối/khóa, mất kết nối, kill ứng dụng và mất điện trên ổ
  NTFS/ReFS dùng thử hoặc ảnh chụp VM.
- [ ] Đánh giá bảo mật độc lập, ký Authenticode và xác minh nhà phát hành.

Kho mã và bộ cài chưa ký được chấp nhận cho phát triển/kiểm thử cục bộ. Các cổng bên ngoài chưa đạt
chặn phát hành công khai có chữ ký; chúng không vô hiệu thao tác xóa cục bộ rõ ràng hoặc làm yếu các
bảo đảm quét/cách ly/khôi phục đã chứng minh.
