#!/bin/sh
set -eu

database="${SAFE_DEDUPE_DATABASE:-/data/state.db}"
log_directory="${SAFE_DEDUPE_LOG_DIRECTORY:-/data/logs}"
source_root="${SAFE_DEDUPE_SOURCE_ROOT:-/scan}"
mode="${SAFE_DEDUPE_MODE:-scan}"
binary="${SAFE_DEDUPE_BINARY:-/usr/local/bin/safe-dedupe}"
command_name="${1:-check}"
subcommand_name="${2:-}"

mount_options="$(findmnt -T "$source_root" -n -o OPTIONS 2>/dev/null || true)"
if [ -z "$mount_options" ]; then
  echo "Từ chối khởi động: thư mục nguồn không nằm trên mount có thể phát hiện: $source_root" >&2
  exit 3
fi

has_mount_option() {
  case ",$mount_options," in
    *,$1,*) return 0 ;;
    *) return 1 ;;
  esac
}

is_mutation_command() {
  case "$command_name:$subcommand_name" in
    quarantine:apply|restore:*|recover:reconcile) return 0 ;;
    *) return 1 ;;
  esac
}

is_permanent_delete_command() {
  case "$command_name:$subcommand_name" in
    quarantine:delete-prepare|quarantine:delete-execute) return 0 ;;
    *) return 1 ;;
  esac
}

if is_permanent_delete_command; then
  echo "Từ chối xóa vĩnh viễn: bản dựng container không bao giờ cho phép thao tác không thể hoàn tác." >&2
  exit 3
fi

case "$mode" in
  scan)
    if ! has_mount_option ro; then
      echo "Từ chối chế độ quét: hãy mount dữ liệu nguồn ở chế độ chỉ đọc (:ro)." >&2
      exit 3
    fi
    if is_mutation_command; then
      echo "Từ chối thay đổi trong chế độ quét; đặt SAFE_DEDUPE_MODE=quarantine và mount /scan đọc-ghi." >&2
      exit 3
    fi
    ;;
  quarantine)
    if ! has_mount_option rw; then
      echo "Từ chối chế độ cách ly: cần mount nguồn đọc-ghi rõ ràng." >&2
      exit 3
    fi
    quarantine_root="${SAFE_DEDUPE_QUARANTINE_ROOT:-}"
    if [ -z "$quarantine_root" ]; then
      echo "Từ chối chế độ cách ly: bắt buộc có SAFE_DEDUPE_QUARANTINE_ROOT." >&2
      exit 3
    fi
    case "$quarantine_root/" in
      "$source_root"/*) ;;
      *)
        echo "Từ chối thư mục cách ly ngoài mount nguồn; di chuyển portable phải nằm trên cùng hệ thống tệp." >&2
        exit 3
        ;;
    esac
    case "$command_name:$subcommand_name" in
      quarantine:*|restore:*|recover:inspect|recover:reconcile|check:*) ;;
      *)
        echo "Từ chối quy trình không cách ly khi mount nguồn ở chế độ đọc-ghi." >&2
        exit 3
        ;;
    esac
    if [ "$command_name:$subcommand_name" = "quarantine:apply" ]; then
      has_root_argument=false
      for argument in "$@"; do
        if [ "$argument" = "--quarantine-root" ]; then
          has_root_argument=true
        fi
      done
      if [ "$has_root_argument" = false ]; then
        set -- "$@" --quarantine-root "$quarantine_root"
      fi
    fi
    ;;
  *)
    echo "SAFE_DEDUPE_MODE phải là scan hoặc quarantine." >&2
    exit 2
    ;;
esac

exec "$binary" \
  --database "$database" \
  --log-directory "$log_directory" \
  "$@"
