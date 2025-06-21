@echo off
setlocal enabledelayedexpansion
set "SCRIPT_VERSION=2.1.0"
set "MIN_POWERSHELL_VERSION=5.0"
set "MIN_VS_VERSION=17.0"
set "VULKAN_SDK_VERSION=1.3.268"

echo ===============================================
echo Vulkan Renderer Engine Installer v%SCRIPT_VERSION%
echo ===============================================

call :check_admin
call :detect_architecture
call :verify_powershell
call :check_prerequisites
call :install_package_managers
call :install_vulkan_sdk
call :install_visual_studio_tools
call :install_rust_toolchain
call :configure_development_environment
call :build_project
call :verify_installation

echo.
echo ===============================================
echo Installation completed successfully!
echo ===============================================
pause
exit /b 0

:check_admin
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Administrator privileges required
    echo Please run as Administrator
    pause
    exit /b 1
)
echo [OK] Administrator privileges detected
goto :eof

:detect_architecture
set "ARCH=unknown"
if "%PROCESSOR_ARCHITECTURE%"=="AMD64" set "ARCH=x64"
if "%PROCESSOR_ARCHITECTURE%"=="x86" set "ARCH=x86"
if "%PROCESSOR_ARCHITECTURE%"=="ARM64" set "ARCH=arm64"

if "%ARCH%"=="unknown" (
    echo ERROR: Unsupported architecture: %PROCESSOR_ARCHITECTURE%
    exit /b 1
)

echo [OK] Architecture detected: %ARCH%
goto :eof

:verify_powershell
powershell -Command "if ($PSVersionTable.PSVersion.Major -lt 5) { exit 1 }" >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: PowerShell 5.0+ required
    echo Please update Windows or install PowerShell Core
    exit /b 1
)
echo [OK] PowerShell version verified
goto :eof

:check_prerequisites
echo Checking system prerequisites...

where git >nul 2>&1
if %errorlevel% neq 0 (
    echo WARNING: Git not found, installing via winget...
    call :install_git
)

where cmake >nul 2>&1
if %errorlevel% neq 0 (
    echo WARNING: CMake not found, will install via Visual Studio
)

echo [OK] Prerequisites check completed
goto :eof

:install_package_managers
echo Installing package managers...

where winget >nul 2>&1
if %errorlevel% neq 0 (
    echo Installing Windows Package Manager...
    powershell -Command "& {
        $progressPreference = 'silentlyContinue'
        Invoke-WebRequest -Uri 'https://aka.ms/getwinget' -OutFile '%TEMP%\winget.msixbundle'
        Add-AppxPackage '%TEMP%\winget.msixbundle'
        Remove-Item '%TEMP%\winget.msixbundle'
    }"
    if !errorlevel! neq 0 (
        echo ERROR: Failed to install winget
        exit /b 1
    )
)

where choco >nul 2>&1
if %errorlevel% neq 0 (
    echo Installing Chocolatey...
    powershell -Command "Set-ExecutionPolicy Bypass -Scope Process -Force; [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072; iex ((New-Object System.Net.WebClient).DownloadString('https://community.chocolatey.org/install.ps1'))"
    if !errorlevel! neq 0 (
        echo WARNING: Chocolatey installation failed, continuing without it
    )
)

echo [OK] Package managers configured
goto :eof

:install_git
winget install --id Git.Git --exact --silent --accept-package-agreements --accept-source-agreements
if %errorlevel% neq 0 (
    choco install git -y
    if !errorlevel! neq 0 (
        echo ERROR: Failed to install Git
        exit /b 1
    )
)
echo [OK] Git installed successfully
goto :eof

:install_vulkan_sdk
echo Installing Vulkan SDK...

reg query "HKLM\SOFTWARE\Khronos\Vulkan\SDK" >nul 2>&1
if %errorlevel% equ 0 (
    echo [OK] Vulkan SDK already installed
    goto :eof
)

echo Downloading Vulkan SDK %VULKAN_SDK_VERSION%...
set "VULKAN_INSTALLER=%TEMP%\VulkanSDK-installer.exe"

powershell -Command "& {
    $progressPreference = 'silentlyContinue'
    $url = 'https://sdk.lunarg.com/sdk/download/%VULKAN_SDK_VERSION%/windows/VulkanSDK-%VULKAN_SDK_VERSION%-Installer.exe'
    Invoke-WebRequest -Uri $url -OutFile '%VULKAN_INSTALLER%'
}"

if not exist "%VULKAN_INSTALLER%" (
    echo ERROR: Failed to download Vulkan SDK
    exit /b 1
)

echo Installing Vulkan SDK silently...
"%VULKAN_INSTALLER%" --accept-licenses --default-answer --confirm-command install
if %errorlevel% neq 0 (
    echo ERROR: Vulkan SDK installation failed
    exit /b 1
)

del "%VULKAN_INSTALLER%"
echo [OK] Vulkan SDK installed successfully
goto :eof

:install_visual_studio_tools
echo Configuring Visual Studio Build Tools...

where cl >nul 2>&1
if %errorlevel% equ 0 (
    echo [OK] MSVC compiler already available
    goto :eof
)

winget list "Microsoft.VisualStudio.2022.BuildTools" >nul 2>&1
if %errorlevel% neq 0 (
    echo Installing Visual Studio Build Tools 2022...
    winget install Microsoft.VisualStudio.2022.BuildTools --silent --override "--wait --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.CMake.Project --add Microsoft.VisualStudio.Component.Windows11SDK.22621"
    if !errorlevel! neq 0 (
        echo ERROR: Failed to install Visual Studio Build Tools
        exit /b 1
    )
)

call "%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
if %errorlevel% neq 0 (
    call "%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
    if !errorlevel! neq 0 (
        echo WARNING: Could not initialize MSVC environment
    )
)

echo [OK] Visual Studio Build Tools configured
goto :eof

:install_rust_toolchain
echo Installing Rust toolchain...

where rustc >nul 2>&1
if %errorlevel% equ 0 (
    echo [OK] Rust already installed, updating...
    rustup update stable
    goto configure_rust
)

echo Downloading rustup-init...
set "RUSTUP_INSTALLER=%TEMP%\rustup-init.exe"

powershell -Command "& {
    $progressPreference = 'silentlyContinue'
    Invoke-WebRequest -Uri 'https://win.rustup.rs' -OutFile '%RUSTUP_INSTALLER%'
}"

if not exist "%RUSTUP_INSTALLER%" (
    echo ERROR: Failed to download rustup-init
    exit /b 1
)

echo Installing Rust...
"%RUSTUP_INSTALLER%" -y --default-toolchain stable --profile complete
if %errorlevel% neq 0 (
    echo ERROR: Rust installation failed
    exit /b 1
)

del "%RUSTUP_INSTALLER%"

set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

:configure_rust
echo Configuring Rust toolchain...

rustup default stable
rustup component add clippy rustfmt llvm-tools-preview
rustup target add wasm32-unknown-unknown

cargo install cargo-audit cargo-deny cargo-outdated

echo [OK] Rust toolchain configured successfully
goto :eof

:configure_development_environment
echo Configuring development environment...

if not exist ".cargo" mkdir .cargo
echo [build] > .cargo\config.toml
echo target-dir = "target" >> .cargo\config.toml
echo [target.x86_64-pc-windows-msvc] >> .cargo\config.toml
echo linker = "link.exe" >> .cargo\config.toml

if not exist ".vscode" mkdir .vscode
echo { > .vscode\settings.json
echo   "rust-analyzer.cargo.target": "x86_64-pc-windows-msvc", >> .vscode\settings.json
echo   "rust-analyzer.checkOnSave.command": "clippy" >> .vscode\settings.json
echo } >> .vscode\settings.json

echo [OK] Development environment configured
goto :eof

:build_project
echo Building Vulkan Renderer Engine...

if not exist "Cargo.toml" (
    echo WARNING: Cargo.toml not found, skipping build
    goto :eof
)

echo Running cargo check...
cargo check --all-features
if %errorlevel% neq 0 (
    echo ERROR: Cargo check failed
    exit /b 1
)

echo Building release version...
cargo build --release --all-features
if %errorlevel% neq 0 (
    echo ERROR: Build failed
    exit /b 1
)

echo [OK] Build completed successfully
goto :eof

:verify_installation
echo Verifying installation...

where rustc >nul 2>&1 && echo [OK] Rust compiler available || echo [ERROR] Rust compiler missing

where cargo >nul 2>&1 && echo [OK] Cargo available || echo [ERROR] Cargo missing

reg query "HKLM\SOFTWARE\Khronos\Vulkan\SDK" >nul 2>&1 && echo [OK] Vulkan SDK registered || echo [ERROR] Vulkan SDK missing

where cl >nul 2>&1 && echo [OK] MSVC compiler available || echo [WARNING] MSVC compiler not in PATH

if exist "target\release\vulkan-renderer.exe" (
    echo [OK] Project executable built successfully
) else (
    echo [WARNING] Project executable not found
)

echo [OK] Installation verification completed
goto :eof