# Phục hồi và sao lưu thủ công

## Thành phần dữ liệu

- Desktop: `state.db`, `state.db-wal`, `state.db-shm`, `state.transactions.jsonl` và `logs/` trong thư
  mục dữ liệu cục bộ của ứng dụng Tauri.
- CLI: đường dẫn cơ sở dữ liệu truyền bằng `--database`, các tệp WAL/SHM bên cạnh, tệp
  `.transactions.jsonl` và `logs/`.
- Vùng cách ly: `.safe-duplicate-finder-quarantine/<project>/<session>/<item>/...` trên từng ổ nguồn.

## Sao lưu an toàn

Dừng tiến trình desktop/CLI trước. Sao chép cơ sở dữ liệu cùng mọi tệp `-wal`, `-shm`, manifest giao
dịch, nhật ký và toàn bộ thư mục cách ly. Không chỉ chép `state.db` khi tiến trình ghi còn hoạt động.
Hãy giữ ACL và dấu thời gian nếu có thể.

Để tạo bản sao lưu SQLite nhất quán khi ứng dụng hoạt động, đặt đích ngoài mọi thư mục nguồn/cách ly.
Lệnh này checkpoint WAL, dùng SQLite `VACUUM INTO`, đồng bộ kết quả và từ chối ghi đè:

```powershell
safe-dedupe --database D:\SafeDedupe\state.db backup `
  --destination E:\SafeDedupeBackups\state-2026-07-22.db
```

## Giao dịch bị gián đoạn

1. Chưa tự di chuyển hoặc xóa bất kỳ tệp nào.
2. Sao lưu cơ sở dữ liệu/manifest/vùng cách ly.
3. Mở Phục hồi → Kiểm tra hoặc chạy `recover inspect --project <UUID>`.
4. Nếu chỉ có đích, đối soát sẽ băm và xác minh đầy đủ đích trước khi ghi nối `verified`.
5. Nếu chỉ có nguồn, chưa quan sát thấy dữ liệu bị di chuyển; hãy giữ nó và xem lại kế hoạch.
6. Nếu cả hai tồn tại, giữ cả hai và so sánh độc lập. Không để công cụ tự chọn.
7. Nếu cả hai đều mất, dừng ghi lên ổ đĩa và dùng bản sao lưu/công cụ phục hồi hệ thống tệp.

## Kiểm tra tính toàn vẹn

Ứng dụng chạy `PRAGMA quick_check` khi mở cơ sở dữ liệu. Chẩn đoán ngoại tuyến bằng SQLite CLI:

```text
PRAGMA integrity_check;
PRAGMA foreign_key_check;
SELECT * FROM file_transactions WHERE status NOT IN ('verified','cancelled','preflight_failed');
SELECT * FROM transaction_events ORDER BY sequence;
```

Không bao giờ cập nhật/xóa `transaction_events` hoặc `audit_events`; trigger chủ đích từ chối thao tác
đó.
