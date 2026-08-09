# Báo cáo đo hiệu năng

## An toàn và cách diễn giải

Bộ tạo dữ liệu đo hiệu năng tách **kích thước tham chiếu logic** khỏi **số byte được ghi vật lý**. Kế hoạch mặc
định biểu diễn 100.000 tệp nhỏ, 10.000 PDF và 88 tệp lớn luân phiên từ 1 đến 20 GiB: tổng
1.026.122.317.824 byte logic (khoảng 1,026 TB thập phân). Công cụ không ghi tệp nguồn nếu thiếu
`--materialize` và từ chối ghi vượt `--max-materialized-bytes`.

Trình chạy chỉ đọc. Nó liệt kê mà không đi theo liên kết, giữ tối đa `--hash-limit` đường dẫn, thực
hiện riêng hai lượt BLAKE3 và SHA-256 đầy đủ, rồi báo peak working set trên Windows.

## Kiểm thử nhanh bản phát hành cục bộ — 2026-07-22

Đây là smoke công cụ/phép đo, không phải qualification 1 TB trên thiết bị tham chiếu bắt buộc.

| Chỉ số | Kết quả |
|---|---:|
| Số tệp vật lý | 32 |
| Số byte vật lý | 262.144 |
| Số byte logic của kịch bản trong manifest | 3.305.439.232 |
| Liệt kê | 0,002471 giây |
| Số tệp được băm kép | 32 |
| Tổng byte băm (hai lượt) | 524.288 |
| Băm | 0,0190839 giây |
| Thông lượng băm tổng hợp | 26,2001 MiB/giây |
| Peak working set | 5.988.352 byte |
| Lỗi | 0 |

Dữ liệu mẫu chủ đích rất nhỏ nên cache/khởi động chi phối; không được ngoại suy thông lượng lên 1 TB.

## Cách tái lập

Chỉ tạo manifest logic an toàn lớp 1 TB, không tạo tệp dữ liệu:

```powershell
cargo run --release -p safe-dedupe-benchmarks --bin generate-benchmark -- `
  --destination D:\BenchmarkData `
  --manifest D:\BenchmarkResults\reference-plan.json
```

Ghi một dữ liệu smoke có giới hạn rồi chạy:

```powershell
cargo run --release -p safe-dedupe-benchmarks --bin generate-benchmark -- `
  --destination D:\BenchmarkData\smoke `
  --manifest D:\BenchmarkResults\smoke-manifest.json `
  --small-files 20 --pdf-files 10 --large-files 2 `
  --materialize --bytes-per-file 8192 --max-materialized-bytes 1048576

cargo run --release -p safe-dedupe-benchmarks --bin run-benchmark -- `
  --root D:\BenchmarkData\smoke --hash-limit 32
```

Để đặc trưng nhiều ổ, lặp `--destination` khi tạo và `--root` khi chạy với thư mục gốc rõ ràng trên
từng ổ thử nghiệm. Ghi lại model ổ, hệ thống tệp, dung lượng trống, phiên bản Windows, power plan,
trạng thái antivirus và cache nóng/lạnh.

## Các bước thẩm định phát hành còn cần

- Chạy toàn bộ tập tham chiếu trên máy Windows i5-14400F/32 GB mục tiêu.
- Thử riêng HDD, SATA SSD, NVMe và bố cục nhiều ổ.
- Ghi lượt lạnh/nóng, kích thước SQLite, thời gian thực, thông lượng đĩa, CPU và peak memory.
- Xác nhận peak working set dưới mục tiêu 2 GB và kiểm tra mọi lỗi bị cô lập.
