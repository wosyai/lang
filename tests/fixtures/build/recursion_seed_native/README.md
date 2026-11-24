# Inspect the Windows stack

From the built fixture’s project directory:

```sh
llvm-readobj --file-headers .wosy/artifacts/app/dev/main/main
```

Check the reserve and commit sizes against [build-native](project/scripts/build-native).
