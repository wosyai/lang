; ModuleID = 'top_level_storage'
source_filename = "top_level_storage"

@top_level_storage__src_main_w__global_observed = external global i32
@top_level_storage__src_storage_w__global_count = external global i32

define i32 @top_level_storage__src_storage_w__read_count() {
entry:
  %count = load i32, ptr @top_level_storage__src_storage_w__global_count, align 4
  ret i32 %count
}

define i32 @main() {
entry:
  %call = call i32 @top_level_storage__src_storage_w__read_count()
  store i32 %call, ptr @top_level_storage__src_main_w__global_observed, align 4
  store i32 42, ptr @top_level_storage__src_storage_w__global_count, align 4
  ret i32 0
}
