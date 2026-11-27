; El instalador de Windows (NSIS).
;
; No lleva escrito ni el nombre, ni el identificador, ni la versión: los recibe
; de `package.ps1` como definiciones (/D…), que los lee de
; `packaging/identity.env` y de `Cargo.toml`. Lo comprueba `identity::tests`.
;
; Instala **por usuario** (%LOCALAPPDATA%\Programs), sin pedir permisos de
; administrador: la app no toca nada del sistema, y un instalador sin firmar
; que además pide elevación asusta dos veces. Al lado de la app de Tauri no
; choca: otra carpeta, otra entrada de desinstalación (la clave es el
; identificador) y otro acceso directo.
;
; SIN FIRMAR. No hay certificado de firma de código en este proyecto, así que
; SmartScreen avisará («Windows protegió su PC» → Más información → Ejecutar
; de todas formas). Cuando lo haya, se firman el .exe de la app antes de
; empaquetar y el instalador después, en `package.ps1`.

Unicode true
!include "MUI2.nsh"

!ifndef APP_NAME | APP_ID | APP_BINARY | APP_VERSION | SOURCE_EXE | LICENSES | ICON_FILE | OUT_FILE
  !error "Faltan definiciones: este script se lanza desde package.ps1"
!endif

; El esquema cac:// por usuario (HKCU\Software\Classes): sin administrador, y
; sin tocar el de nadie más. Windows lanza el comando con el enlace en %1.
!define SCHEME_KEY "Software\Classes\cac"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"

Name "${APP_NAME}"
OutFile "${OUT_FILE}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
; Al actualizar, la carpeta donde ya estaba.
InstallDirRegKey HKCU "Software\${APP_ID}" "InstallDir"
SetCompressor /SOLID lzma

!define MUI_ICON "${ICON_FILE}"
!define MUI_UNICON "${ICON_FILE}"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_BINARY}.exe"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; El primero es el de reserva cuando Windows no está en ninguno de los dos.
!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"
  File "/oname=${APP_BINARY}.exe" "${SOURCE_EXE}"
  SetOutPath "$INSTDIR\licenses"
  File "${LICENSES}"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${APP_BINARY}.exe"

  WriteRegStr HKCU "${SCHEME_KEY}" "" "URL:cac"
  WriteRegStr HKCU "${SCHEME_KEY}" "URL Protocol" ""
  WriteRegStr HKCU "${SCHEME_KEY}\DefaultIcon" "" "$INSTDIR\${APP_BINARY}.exe,0"
  WriteRegStr HKCU "${SCHEME_KEY}\shell\open\command" "" '"$INSTDIR\${APP_BINARY}.exe" "%1"'

  WriteRegStr HKCU "Software\${APP_ID}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_BINARY}.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Guz Studio"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

; Desinstalar quita la app, no lo de la persona: la sesión y las preferencias
; (%APPDATA%) se quedan, como al desinstalar cualquier otra cosa.
Section "Uninstall"
  Delete "$INSTDIR\${APP_BINARY}.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir /r "$INSTDIR\licenses"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"
  DeleteRegKey HKCU "${SCHEME_KEY}"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\${APP_ID}"
SectionEnd
