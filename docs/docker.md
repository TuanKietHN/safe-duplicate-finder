# Chế độ Docker / không giao diện

Docker là bề mặt phụ. Chế độ quét mặc định chỉ đọc; thay đổi dữ liệu chỉ khả dụng qua chế độ cách ly
rõ ràng và mount nguồn đọc-ghi. Image chạy bằng UID 10001 (`safe-dedupe`); entrypoint từ chối khi chế
độ và mount không khớp trước khi CLI khởi động.

## Chế độ quét chỉ đọc

```bash
docker build -t safe-dedupe .
docker run --rm \
  -v /host/library:/scan:ro \
  -v safe-dedupe-data:/data \
  -v /host/reports:/reports \
  -e SAFE_DEDUPE_MODE=scan \
  safe-dedupe check
```

Chế độ `scan` yêu cầu `/scan` là mount có tùy chọn `ro` và từ chối `quarantine apply`, `restore`,
`recover reconcile`. Mount đọc-ghi bị từ chối kể cả với `check`, nên lệnh sau không thể âm thầm biến
container quét thành container thay đổi dữ liệu.

## Chế độ cách ly rõ ràng

Chỉ dùng sau khi đã tạo và xem xét kế hoạch khóa. Thư mục cách ly phải nằm trong `/scan` để Linux có
thể gọi `renameat2(RENAME_NOREPLACE)` trong cùng hệ thống tệp; không có phương án dự phòng sao chép rồi
xóa.

```bash
docker run --rm \
  -v /host/library:/scan:rw \
  -v safe-dedupe-data:/data \
  -e SAFE_DEDUPE_MODE=quarantine \
  -e SAFE_DEDUPE_QUARANTINE_ROOT=/scan/.safe-duplicate-finder-quarantine \
  safe-dedupe quarantine apply --plan PLAN_UUID --confirm QUARANTINE
```

Entrypoint tự thêm `--quarantine-root` từ biến môi trường nếu lệnh chưa có. Chế độ cách ly từ chối
mount chỉ đọc, thư mục cách ly bị thiếu/nằm ngoài cây và quy trình quét/dự án khi nguồn đang đọc-ghi.
Khôi phục và đối soát phục hồi rõ ràng dùng cùng chế độ. Docker không bao giờ cho phép xóa vĩnh viễn.

## Cấu hình và giới hạn

Image cung cấp `SAFE_DEDUPE_DATABASE`, `SAFE_DEDUPE_LOG_DIRECTORY`, `SAFE_DEDUPE_SOURCE_ROOT`,
`SAFE_DEDUPE_REPORT_DIRECTORY`, `SAFE_DEDUPE_MODE` và `SAFE_DEDUPE_QUARANTINE_ROOT`. Giá trị mặc định
lần lượt là `/data/state.db`, `/data/logs`, `/scan`, `/reports` và `scan`; thư mục cách ly không có mặc
định và phải được cung cấp rõ ràng.

Danh tính Linux là `(device, inode)`, được kiểm tra trước và sau thao tác đổi tên không thay thế của
kernel. Cơ chế này không mạnh bằng thao tác Windows gắn với handle `FILE_ID_INFO`, vì vậy desktop/CLI
Windows gốc vẫn là bề mặt thay đổi được khuyến nghị. Bind mount của Docker Desktop có thể dùng hành vi
inode/mount do VM cung cấp; hãy kiểm thử đúng storage driver bằng dữ liệu dùng thử trước. Mount
mạng/đám mây có thể ngắt kết nối hoặc thay đổi khi đọc nên sẽ thất bại đóng và không được chứng minh.

Chạy `tests/container/smoke.ps1` khi có Docker daemon. Script dựng image, xác minh người dùng không
phải root, chứng minh chế độ quét `ro` thành công và `rw` bị từ chối, chứng minh chế độ cách ly từ chối
`ro`, đồng thời xác nhận nội dung nguồn không đổi. Logic entrypoint còn có test shell không cần daemon
tại `tests/container/entrypoint_test.sh`.
