# Thuật toán phát hiện tệp trùng lặp

## Chế độ nghiêm ngặt (mặc định)

Một nhóm chỉ được xác nhận trùng khi đạt mọi điều kiện sau:

1. Tên tệp cuối đường dẫn sau chuẩn hóa Unicode/chữ hoa-thường giống nhau.
2. Kích thước byte 64-bit chính xác giống nhau.
3. Các mẫu BLAKE3 đầu/giữa/cuối có tách miền giống nhau. Bước này chỉ có thể loại trừ.
4. BLAKE3 đầy đủ dạng luồng giống nhau và siêu dữ liệu ổn định trước/sau lượt đọc.
5. SHA-256 đầy đủ dạng luồng giống nhau và siêu dữ liệu ổn định trước/sau lượt đọc.
6. Mỗi thành viên giữ lại biểu diễn một danh tính `(volume_id, file_id 128-bit)` riêng biệt.

Chế độ chỉ so sánh nội dung bỏ điều kiện 1 và không đổi điều kiện nào khác. Ứng dụng không bao giờ tự
chọn chế độ này.

## Lấy mẫu

Tệp tối đa 64 KiB được lấy mẫu một lần. Tệp lớn hơn lấy tối đa 64 KiB ở đầu, chính giữa và cuối. Giá
trị băm chứa ngữ cảnh dẫn xuất, tổng kích thước, offset, độ dài và byte. Mẫu bằng nhau không chứng minh
nội dung bằng nhau; nó chỉ tránh đọc đầy đủ khi có sai khác rõ ràng.

## Băm đầy đủ và độ ổn định

BLAKE3 và SHA-256 chạy thành hai lượt dạng luồng riêng biệt với bộ đệm dùng lại 1 MiB. Checkpoint phối
hợp được kiểm tra ở mỗi khối. Token ảnh chụp trước/sau chứa danh tính vật lý, kích thước và thời điểm
sửa đổi. Đường dẫn thay đổi hoặc danh tính bị thay thế là không ổn định và bị loại.

## Chọn tệp giữ lại

Thứ tự mặc định là thư mục ưu tiên, thời điểm sửa đổi cũ nhất, đường dẫn ngắn nhất, rồi thứ tự từ điển
của đường dẫn. Chính sách thay thế gồm cũ nhất/mới nhất/ngắn nhất. Mỗi nhóm phải có ít nhất một thành
viên `keep` trước khi được khóa. Trigger cơ sở dữ liệu độc lập từ chối kế hoạch khóa không có tệp giữ.

Số byte có thể thu hồi tối đa của nhóm là `size × (independent_members - 1)`. Số byte cách ly thực tế
chỉ được cộng sau khi xác minh đích.

## Liên kết và tệp đặc biệt

Không đi theo symlink/junction thư mục. Tệp symlink bị bỏ qua. Các đường dẫn chung một danh tính vật
lý là bí danh hard link, không phải bản sao độc lập có thể thu hồi. Tệp có số liên kết native lớn hơn
một bị loại kể cả khi bí danh khác nằm ngoài thư mục đã chọn; điều này ngăn ước tính hoặc cách ly coi
phân bổ có liên kết ngoài là bản sao độc lập. Tệp bị khóa, từ chối truy cập, ngoại tuyến, bị thiếu hoặc
đang thay đổi được cô lập thành lỗi thay vì hạ điều kiện chứng minh.
