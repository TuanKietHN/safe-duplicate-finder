# Mô hình đe dọa mất dữ liệu

Tài sản cần bảo vệ nhất là bản sao hợp lệ duy nhất của tài liệu người dùng. Khả năng truy cập và phục
hồi quan trọng hơn dung lượng thu hồi hoặc tốc độ quét.

| Mối đe dọa | Kiểm soát phòng ngừa | Phát hiện/phục hồi |
|---|---|---|
| Va chạm băm/mẫu | Hai giá trị băm đầy đủ độc lập; mẫu không bao giờ là bằng chứng dương | Lưu bằng chứng theo từng ảnh chụp |
| Tệp đổi khi đọc | Token danh tính/kích thước/thời gian trước và sau | Đánh dấu không ổn định; không lập nhóm/thao tác |
| Đường dẫn bị thay sau quét | Đọc danh tính/token từ handle mở không theo liên kết; đổi tên chính handle đã xác minh | `preflight_failed`; không chạm đường dẫn thay thế |
| Bí danh hard link bị tính hai lần | Khử trùng theo ổ Windows + mã tệp 128-bit | Loại bí danh khỏi nhóm/tổng thu hồi |
| Xung đột đích | Kiểm tra tồn tại rõ ràng và đổi tên Windows không thay thế | `move_failed`; không ghi đè |
| Sao chép dở giữa ổ | Cách ly theo từng ổ nguồn và kiểm tra cùng ổ | Di chuyển thất bại đóng |
| Crash trước/sau đổi tên | Ý định/sự kiện bền vững trước thay đổi; `moved_unverified` sau đó | Khi khởi động kiểm tra cả hai đường dẫn |
| Đích hỏng | BLAKE3 + SHA-256 đầy đủ sau đổi tên | `verify_failed` → `recovery_required` |
| Mất bản giữ cuối | Trigger kế hoạch + kiểm tra danh tính/băm đầy đủ trực tiếp trước mỗi mục | Chặn thay đổi |
| Không commit được CSDL/nhật ký | Append+fsync JSONL trước giao dịch SQLite FULL trạng thái+kiểm toán | Dừng trước ranh giới kế; sự kiện manifest dư là bằng chứng phục hồi, không bao giờ có di chuyển thiếu nhật ký |
| Văn bản báo cáo độc hại | HTML đã escape; thư viện CSV quote | Không có nội dung tài liệu thực thi được |
| Xóa vĩnh viễn nhầm | Chỉ UUID cách ly đã xác minh; chế độ thường phải hết thời hạn; chọn hàng loạt chỉ áp dụng cho mục đủ điều kiện đang hiển thị; không tự dọn; token một lần và câu chính xác | Ý định từng mục bền vững, kiểm tra trước hai băm đầy đủ, phục hồi dừng ở lỗi đầu; cổng riêng |

## Bất biến không thể thỏa hiệp

- Quét và chạy thử không có đường dẫn thay đổi hệ thống tệp.
- Mẫu trùng không bao giờ được hiển thị là tệp trùng.
- Nhóm đã khóa luôn giữ lại một tệp.
- Không thể bắt đầu thay đổi nếu chưa có ý định manifest bền vững và commit SQLite trạng thái/kiểm toán
  tương ứng.
- Không ghi đè, không dự phòng sao chép giữa ổ, không tự động xóa vĩnh viễn; xóa rõ ràng chỉ dành cho
  mục cách ly đã xác minh và hết thời hạn lưu giữ.
- Số byte thu hồi/cách ly chỉ tăng sau khi xác minh đích.
- Lịch sử sự kiện chỉ ghi nối không bao giờ bị viết lại khi phục hồi.

## Rủi ro còn lại

- Hỏng phần cứng lưu trữ, kernel hoặc hệ thống tệp có thể vô hiệu bảo đảm bên dưới API hệ điều hành.
- Mã độc có cùng quyền người dùng có thể sửa nguồn, cách ly, cơ sở dữ liệu hoặc nhật ký.
- Ngữ nghĩa placeholder mạng/đám mây khác nhau; tệp không khả dụng bị bỏ qua thay vì tự tải xuống.
- Nguồn có thể đổi ngay sau kiểm tra tệp giữ lại; nguồn được cách ly vẫn trải qua kiểm tra
  danh tính/token từ handle riêng, còn trường hợp tệp giữ mơ hồ yêu cầu phục hồi.

Vẫn cần bản sao lưu. Ứng dụng là công cụ quản lý tệp trùng lặp, không phải hệ thống sao lưu.
