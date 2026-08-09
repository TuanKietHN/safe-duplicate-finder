# Hướng dẫn sử dụng

## 1. Dự án và thư mục

Tạo một dự án. Hãy giữ chế độ **Nghiêm ngặt** trừ khi bạn chủ động muốn nhóm các tệp có nội dung
giống nhau nhưng khác tên. Thêm mọi thư mục trước khi quét; việc thêm thư mục không tự động bắt đầu
quét. Ứng dụng từ chối thư mục bị thiếu, thư mục cách ly, thư mục trùng/chồng lấn cha-con và thư mục
chứa cơ sở dữ liệu.

Quét nghiêm ngặt mặc định xử lý PDF, EPUB và MOBI. Bộ lọc dự án được giữ qua lần khởi động lại, hỗ
trợ phần mở rộng cần quét/loại trừ, mẫu glob cần quét/loại trừ, kích thước tối thiểu và lựa chọn tệp
ẩn/hệ thống. Hãy kiểm tra và lưu trước khi quét. **Mọi tệp** là tùy chọn rõ ràng cho từng phiên quét;
nó không âm thầm sửa bộ lọc của dự án.

## 2. Quét

Chọn **Bắt đầu quét chỉ đọc**. Yêu cầu tạm dừng/tiếp tục/hủy được xử lý giữa các tệp và từng khối băm.
Lỗi do tệp không thể truy cập hoặc đang thay đổi làm tăng bộ đếm lỗi, không hạ điều kiện chứng minh.
Nếu trạng thái thành `blocked`, hãy kiểm tra nhật ký và phần Phục hồi trước khi thay đổi dữ liệu.

Tóm tắt trực tiếp tách riêng số tệp đã phát hiện, bỏ qua, không ổn định và lỗi. Nó cũng hiển thị số
byte đã đọc, tốc độ đọc, thời gian đã chạy, thời gian còn lại ước tính và số byte có thể thu hồi. Đây
là số liệu quan sát, không phải bằng chứng rằng việc di chuyển đã an toàn.

Chế độ chỉ so sánh nội dung hiển thị ô xác nhận cảnh báo. Chế độ này bỏ qua tên tệp nhưng vẫn yêu cầu
kích thước, hai giá trị băm đầy đủ ổn định và các danh tính vật lý khác nhau.

CLI cung cấp cùng giao thức điều khiển bền vững qua `scan pause`, `scan resume`, `scan cancel` và
`scan status`. Tiến trình CLI thứ hai có thể gửi yêu cầu bằng mã phiên được in ra khi `scan start` bắt
đầu. Luồng đang tạm dừng sẽ dừng tại tệp kế tiếp hoặc ranh giới khối băm 1 MiB; `resume` cũng có thể
khởi động lại phiên chỉ đọc bị gián đoạn sau khi bằng chứng cũ được loại bỏ.

## 3. Kết quả và chạy thử

Khi mở trang này từ một phiên vừa hoàn tất, ứng dụng tự tải các nhóm đã chứng minh và hiển thị tổng
số nhóm, tổng số tệp, số bản sao có thể cách ly cùng dung lượng có thể thu hồi. Nếu mở lại ứng dụng,
hãy dán mã phiên đã hoàn tất vào ô mã phiên; kết quả sẽ tự tải. Mỗi nhóm hiển thị bằng chứng
BLAKE3/SHA-256; mỗi thành viên hiển thị kích thước, thời điểm sửa đổi, vai trò giữ lại/cách ly và lý
do. Chọn chính sách giữ tệp rồi tạo kế hoạch đã khóa. Tổng số chạy thử chỉ là thao tác dự kiến; hệ
thống tệp chưa thay đổi.

Có thể xuất CSV cho bảng tính, JSON cho tự động hóa hoặc HTML đã escape để chia sẻ cục bộ.

## 4. Cách ly

Kiểm tra mã kế hoạch và nhập chính xác `QUARANTINE`. Trước mỗi lần di chuyển, ứng dụng xác minh tệp
giữ lại còn sống và nguồn đã chọn. Ứng dụng tạo vùng cách ly ẩn trên cùng ổ đĩa với nguồn, giữ đường
dẫn tương đối dưới mã dự án/phiên/mục, từ chối ghi đè và băm đích hai lần.

Một mục chỉ xuất hiện ở trạng thái **Đã xác minh** sau khi mọi kiểm tra đạt. Đường dẫn gốc được giải
phóng nhưng dữ liệu vẫn có thể khôi phục từ vùng cách ly.

Danh sách cách ly có thể tìm theo đường dẫn gốc/cách ly và lọc theo trạng thái hoặc ngày cách ly. Các
bộ lọc danh sách không thể vượt qua cổng xác nhận chính xác dành cho thao tác thay đổi.

## 5. Khôi phục

Với mục đã xác minh, nhập chính xác `RESTORE`. Khôi phục từ chối ghi đè nếu đường dẫn gốc đã bị chiếm.
Việc di chuyển và xác minh dùng cùng giao thức nhật ký theo chiều ngược lại. Hãy tự xử lý xung đột tại
đường dẫn gốc trước khi thử lại.

## 6. Phục hồi giao dịch

Kiểm tra phục hồi là chỉ đọc. Với giao dịch bị gián đoạn, kiểm tra đường dẫn nguồn và đích rồi nhập
`RECONCILE`. Bộ máy xác minh đích trước khi đánh dấu hoàn tất; trường hợp cả hai cùng tồn tại hoặc cả
hai cùng mất vẫn cần xử lý thủ công/khẩn cấp.

## 7. Xóa vĩnh viễn khỏi vùng cách ly

Đây là quy trình không thể hoàn tác riêng biệt, không xuất hiện khi quét, chạy thử, cách ly hoặc khôi
phục. Ở chế độ thường, hãy đợi qua ngày lưu giữ hiển thị rồi chọn từng mục đủ điều kiện. Nếu cần bỏ
thời gian giữ 30 ngày, bật riêng **Xóa ngay — bỏ qua thời gian giữ 30 ngày**, rồi chọn từng mục hoặc
dùng **Chọn tất cả … mục đủ điều kiện**. Nút này chỉ chọn các mục khớp bộ lọc đang hiển thị và chưa
xóa dữ liệu.

Kiểm tra số tệp và tổng byte, chọn **Chuẩn bị xác nhận Xóa ngay**, đánh dấu ô
**Tôi hiểu và xác nhận xóa vĩnh viễn**, rồi bấm nút xóa. Ứng dụng tự gửi token/câu kỹ thuật cho
backend; bạn không phải sao chép thủ công. Chỉ chuẩn bị thì chưa xóa tệp. Trước lần xóa đầu tiên, ứng
dụng kiểm tra lại danh tính, kích thước, BLAKE3 và SHA-256 của mọi mục đã chọn. Nếu một mục thất bại,
cả lô dừng.

Lệnh CLI tương đương:

```powershell
safe-dedupe --database <DB> quarantine delete-prepare --entry <UUID> [--entry <UUID> ...]
safe-dedupe --database <DB> quarantine delete-execute --batch <UUID> --token <TOKEN> --confirm "<EXACT_PHRASE>"
```

Thêm `--delete-now` vào lệnh `delete-prepare` để bỏ qua thời gian giữ 30 ngày. Câu xác nhận trả về sẽ
bắt đầu bằng `XÓA NGAY VĨNH VIỄN`; mọi kiểm tra an toàn khác vẫn giữ nguyên.

Bản container luôn từ chối các lệnh này.

## 8. Cài đặt và gỡ cài đặt

Bộ cài online tự kiểm tra WebView2. Nếu Runtime đã hợp lệ, bộ cài bỏ qua tải xuống. Nếu lần tải trước
bị gián đoạn, bộ cài tiếp tục phần `.part` còn thiếu; tệp hoàn chỉnh chỉ được dùng sau khi đúng kích
thước và SHA-256. Cửa sổ tiến độ hiển thị byte đã nhận/tổng byte, tốc độ, ETA, phần trăm chung, tệp hiện
tại và trạng thái từng thành phần. Cache cài đặt nằm tại
`%LOCALAPPDATA%\io.github.safeduplicate.finder\installer-cache`.

Shortcut **Gỡ cài đặt Trình tìm tệp trùng lặp an toàn** xóa chương trình, cơ sở dữ liệu, nhật ký,
thiết lập, cache giao diện/Runtime và shortcut của ứng dụng. Trước khi xác nhận, lưu hoặc xuất lịch sử
nếu còn cần. Bộ gỡ cài đặt không xóa tài liệu đang nằm trong `.safe-duplicate-finder-quarantine` trên
các ổ nguồn vì đó có thể là bản duy nhất còn lại; hãy khôi phục hoặc xóa chúng bằng quy trình trong ứng
dụng trước khi gỡ.

## Nhắc nhở an toàn

- Luôn giữ bản sao lưu độc lập.
- Không sửa hoặc di chuyển tệp trong kế hoạch giữa lúc quét và cách ly.
- Không tự xóa cơ sở dữ liệu hoặc manifest JSONL khi còn giao dịch chờ xử lý.
- Hết thời hạn lưu giữ không bao giờ tự kích hoạt xóa; luôn cần chọn rõ ràng và nhập đủ hai xác nhận.
