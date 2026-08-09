# Kiểm thử và bằng chứng tiêm lỗi

## Cổng tự động

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run test
npm --prefix apps/desktop run build
```

Dữ liệu kiểm thử tích hợp hiện bao phủ:

- cùng tên/kích thước nhưng khác một byte không phải tệp trùng;
- bản sao độc lập chính xác cần cả hai giá trị băm đầy đủ 32 byte;
- tệp rỗng, nhiều khối, sparse logic >4 GiB, hủy và thay đổi trong khi băm;
- đường dẫn Windows Unicode dài và danh tính vật lý native;
- nguồn thay đổi/bị thay thế bị từ chối trước di chuyển, kể cả cửa sổ đua xác minh/đổi tên;
- loại bí danh hard link kể cả khi bí danh khác nằm ngoài thư mục đã chọn;
- đích hỏng không bao giờ đạt `verified`;
- cách ly đã xác minh khôi phục mà không ghi đè;
- SQLite WAL, synchronous FULL, khóa ngoại và mặc định xóa vĩnh viễn bằng 0;
- lỗi manifest-trước-SQLite chặn chuyển trạng thái và thay đổi nguồn;
- trigger SQLite chỉ ghi nối từ chối cập nhật/xóa lịch sử giao dịch/kiểm toán;
- kế hoạch giữ tệp khóa → cách ly có nhật ký → danh sách tìm kiếm được → khôi phục đã xác minh;
- lưu dự án/thư mục gốc/bộ lọc, lưu trữ hồ sơ, sao lưu, tiếp tục checkpoint quét và thời điểm quét;
- từ chối chồng lấn thư mục cha/con;
- sáu ranh giới lỗi xác định từ ý định bền vững đến xác minh đích;
- cổng xác nhận chính xác cho chế độ nội dung/cách ly/khôi phục trong UI;
- xóa vĩnh viễn chỉ trong cách ly: thời hạn, chọn UUID rõ ràng, đúng số lượng/byte, sai token/câu,
  không tự xóa, kiểm tra trước mọi mục, dừng ở lỗi đầu, ý định append/fsync, đối soát gián đoạn sau
  system call, tính lặp an toàn, UI chỉ chọn hàng loạt mục đủ điều kiện và từ chối thay đường dẫn Win32;
- đẩy nhật ký JSONL và văn bản xuống đĩa.

Mốc workspace hiện có 88 test Rust đạt (0 lỗi) và 12 kiểm tra Vitest đạt. Số Rust bao gồm điều khiển
quét bền vững giữa tiến trình, nâng cấp migration-v1, ranh giới tạm dừng/tiếp tục/hủy, vô hiệu bằng
chứng cũ khi mở lại đột ngột, tiêm lỗi khôi phục, khôi phục lô nhóm/phiên/dự án lặp an toàn và cổng an
toàn xóa vĩnh viễn. Clippy nghiêm ngặt, ESLint, build TypeScript/Vite production, Prettier,
`cargo deny`, `cargo audit` và `npm audit` đều hoàn tất thành công. `cargo audit` báo 17 cảnh báo bảo
trì/unsoundness bắc cầu được chấp nhận nhưng không có lỗi lỗ hổng; các cảnh báo được ghi trong bằng
chứng phát hành.

## Phạm vi lỗi tự động và thủ công trước phát hành công khai

Ma trận xác định trong tiến trình hiện tiêm lỗi tại lúc tạo nhật ký, kiểm tra trước, `moving`, đổi tên
hệ thống tệp, `moved_unverified` và xác minh đích. Nó khẳng định luôn còn ít nhất một bản sao và không
ranh giới lỗi nào được tính là đã xác minh. Dữ liệu lỗi manifest cũng chứng minh fsync JSONL thất bại
giữ SQLite ở `planned` và không chạm vào nguồn.

Việc thẩm định phát hành công khai phải mở rộng lên ổ đĩa dùng thử/ảnh chụp máy ảo thực cho cơ sở dữ liệu đầy,
bận/ngắt commit, ổ cách ly đầy, handle bị từ chối/khóa, mất kết nối, kill tiến trình và mất điện. Với
mỗi ranh giới, ghi nhận sự tồn tại nguồn/đích, trạng thái cuối, chuỗi sự kiện, khả năng nhìn thấy trong
danh sách và số byte được tính.

Bộ tự động đã bao phủ đường dẫn Unicode dài, tệp sparse >4 GiB, hard link, xung đột đích và xung đột
khôi phục. Trường hợp phụ thuộc thiết bị còn lại phải dùng ổ/ảnh chụp VM dùng thử — không bao giờ dùng
dữ liệu không thể thay thế.

## Kiểm tra trực quan

Quy trình xem trước trình duyệt kiểm tra bố cục 1280×820, tràn ngang, điều hướng, xác nhận chế độ nội
dung và cổng cách ly phân biệt hoa-thường. Dự án, Quét, Kết quả và Cách ly được kiểm tra lại sau thay
đổi bộ lọc/số liệu cuối; không có tràn ngang hoặc `NaN`/`undefined` được render. Kiểm tra Tauri native
còn phải thử hộp thoại thư mục/lưu và lệnh backend thực.

Chưa tuyên bố chạy image Docker vì Linux daemon của Docker Desktop không khả dụng trên máy kiểm tra.
Đánh giá Dockerfile/entrypoint và test CLI native đạt; smoke thực tế người dùng không phải root với
mount chỉ đọc vẫn là cổng phát hành.
