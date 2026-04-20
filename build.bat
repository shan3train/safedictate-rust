@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set VULKAN_SDK=C:\VulkanSDK\1.4.341.1
set LIBCLANG_PATH=C:\Program Files\LLVM\bin
set CMAKE_GENERATOR=Ninja
set CARGO_TARGET_DIR=C:\sdt
cd /d "%~dp0"
C:\Users\shane\.cargo\bin\cargo build
