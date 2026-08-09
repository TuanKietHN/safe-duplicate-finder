# Cổng xóa vĩnh viễn

Trạng thái: **MỞ CHO PHÁT TRIỂN CỤC BỘ — phát hành công khai/ký số vẫn bị chặn bởi các cổng bên ngoài
bên dưới.**

Cổng kỹ thuật được mở ngày 2026-07-22 sau khi T001–T108 và bộ an toàn/phục hồi tự động đạt. Migration
schema v4 bổ sung hai chế độ xóa và ghi chế độ vào từng lô có nhật ký rõ ràng. Mặc định schema vẫn là
`automatic_permanent_delete = 0`; không timer, quét, tạo dự án, hết hạn lưu giữ hoặc lệnh container
nào có thể bắt đầu xóa.

Bất biến đã triển khai:

- chỉ UUID mục cách ly rõ ràng được vào kiểu miền xóa; kiểu này không có trường đường dẫn gốc/nguồn;
- mục phải còn được xác minh và hoạt động; chế độ thường yêu cầu `retain_until` đã qua;
- bước chuẩn bị hiển thị chính xác số lượng/byte, trả token 10 phút và câu chính xác
  `XÓA VĨNH VIỄN <số_lượng> TỆP ĐÃ CÁCH LY (<số_byte> BYTE) TRONG TRÌNH TÌM TỆP TRÙNG LẶP AN TOÀN 0.2.1`;
- chỉ lưu giá trị băm tách miền của token, gắn với UUID mục đã sắp xếp, dự án, danh tính vật lý, kích
  thước, bằng chứng BLAKE3 và SHA-256;
- mọi mục còn lại được kiểm tra trước danh tính/kích thước/hai băm trước lần xóa mới đầu tiên;
- ý định mục được append và fsync trước thao tác xóa Windows gắn handle; một lỗi dừng lô trước mọi
  đường dẫn khác;
- gián đoạn sau system call xóa để lại ý định `deleting` bền vững. Thử lại cùng lô sẽ đối soát đối
  tượng đã chọn bị thiếu thành đã xóa và có tính lặp an toàn;
- desktop cho chọn riêng từng mục hoặc chọn tất cả mục đủ điều kiện đang hiển thị; lựa chọn hàng loạt
  vẫn chỉ tạo danh sách UUID cho cùng thử thách và không tự động xóa;
- desktop chỉ yêu cầu một ô xác nhận cuối cùng; token và câu kỹ thuật được tự động gửi lại, còn
  backend vẫn kiểm tra khớp tuyệt đối với lô ngắn hạn đã lưu;
- Docker từ chối cả hai lệnh con xóa vĩnh viễn trước khi chạy binary.
- chế độ `Xóa ngay` phải được bật riêng, chỉ bỏ qua cổng thời gian 30 ngày và dùng câu xác nhận riêng
  bắt đầu bằng `XÓA NGAY VĨNH VIỄN`;
- cả hai chế độ vẫn bắt buộc token 10 phút, kiểm tra lại danh tính/kích thước/hai băm, ghi ý định bền
  vững và xóa gắn với handle. Không có tự động xóa.

Bằng chứng tự động tại cổng: 88 test Rust, 12 test Vitest, Clippy nghiêm ngặt, build TypeScript
production, test xóa/thay thế handle Win32 dùng thử thật, trigger sự kiện/kiểm toán chỉ ghi nối, phục
hồi gián đoạn sau system call, kiểm tra trước mọi mục, dừng lô ở lỗi đầu, sai token/câu, thời hạn,
không tự xóa và CLI từ chối dạng đường dẫn nguồn.

Phát hành công khai vẫn cần phê duyệt được ghi nhận cho các qualification bên ngoài:

- ma trận lỗi giao dịch/phục hồi đầy đủ đạt trên ổ NTFS và ReFS dùng thử;
- đánh giá bảo mật/mất dữ liệu độc lập ký bằng chứng phát hành;
- kiểm thử sao lưu/khôi phục và bộ cài trên người dùng sạch đạt.

Đây là năng lực phát triển cục bộ hoàn chỉnh, không phải phê duyệt công bố bộ cài chưa ký. Chỉ dùng dữ
liệu cách ly dùng thử cho đến khi các cổng thiết bị và máy sạch bên ngoài được ký.
