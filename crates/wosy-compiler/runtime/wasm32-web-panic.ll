declare void @llvm.trap()

define void @__wosy_core_system_panic() {
entry:
  call void @llvm.trap()
  unreachable
}
