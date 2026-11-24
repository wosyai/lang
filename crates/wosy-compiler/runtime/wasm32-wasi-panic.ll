define void @__wosy_core_system_panic() {
entry:
  call void @proc_exit(i32 1)
  unreachable
}
