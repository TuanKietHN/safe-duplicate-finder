import type { Mode } from "../types";

const scanStates: Record<string, string> = {
  idle: "Chưa bắt đầu",
  created: "Đã tạo",
  enumerating: "Đang liệt kê tệp",
  metadata: "Đang đọc siêu dữ liệu",
  quick_hashing: "Đang băm nhanh",
  full_hashing: "Đang băm đầy đủ",
  grouping: "Đang lập nhóm",
  paused: "Đã tạm dừng",
  interrupted: "Bị gián đoạn",
  completed: "Hoàn tất",
  cancelled: "Đã hủy",
  blocked: "Bị chặn",
};

const quarantineStates: Record<string, string> = {
  planned: "Đã lên kế hoạch",
  preflight_failed: "Kiểm tra trước thất bại",
  moving: "Đang di chuyển",
  moved_unverified: "Đã chuyển, chưa xác minh",
  verified: "Đã xác minh",
  restoring: "Đang khôi phục",
  restored: "Đã khôi phục",
  recovery_required: "Cần phục hồi",
  deleting: "Đang xóa / cần phục hồi",
  failed: "Xóa thất bại",
  deleted: "Đã xóa vĩnh viễn",
};

const actions: Record<string, string> = {
  keep: "Giữ lại",
  quarantine: "Cách ly",
  manual: "Xem xét thủ công",
};

const reasons: Record<string, string> = {
  "primary root": "Thư mục ưu tiên",
  "duplicate copy": "Bản sao trùng lặp",
};

export const modeLabel = (mode: Mode) =>
  mode === "strict" ? "Nghiêm ngặt" : "Chỉ so sánh nội dung";

export const scanStateLabel = (state: string) => scanStates[state] ?? state;

export const quarantineStateLabel = (state: string) => quarantineStates[state] ?? state;

export const actionLabel = (action: string) => actions[action] ?? action;

export const reasonLabel = (reason: string) => reasons[reason] ?? reason;
