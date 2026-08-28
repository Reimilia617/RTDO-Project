Name:           rtdo
Version:        0.5.0
Release:        1%{?dist}
Summary:        Root Task Do — interactive, policy-driven privilege elevation tool
License:        MIT
URL:            https://github.com/Reimilia617/RTDO-Project
Source0:        rtdo
Source1:        rtdo-sudod
Source2:        rtdo.conf
Source3:        rtdo.pam
Source4:        rtdo-sudod.service
BuildArch:      x86_64
Requires:       pam, systemd

%description
rtdo is a policy-driven, environment-adaptive sudo alternative. It authenticates
against the root password (PAM), then decides per command whether to allow, prompt,
or block based on trusted paths and a blacklist.

On first run it auto-detects a missing or weak root password and guides you through
setting a new one, and asks once whether to disable sudo via a Go daemon
(rtdo-sudod, systemd service) that intercepts `sudo`; use `sudo-force` for the
original sudo. All output is bilingual (Chinese/English), following LANG.

%prep

%build

%install
install -D -m 4755 %{SOURCE0} %{buildroot}/usr/bin/rtdo
install -D -m 0755 %{SOURCE1} %{buildroot}/usr/local/lib/rtdo/rtdo-sudod
install -D -m 0644 %{SOURCE2} %{buildroot}/etc/rtdo/rtdo.conf
install -D -m 0644 %{SOURCE3} %{buildroot}/etc/pam.d/rtdo
install -D -m 0644 %{SOURCE4} %{buildroot}/etc/systemd/system/rtdo-sudod.service

%post
chown root:root /usr/bin/rtdo 2>/dev/null || true
chmod 4755 /usr/bin/rtdo 2>/dev/null || true
if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload >/dev/null 2>&1 || true
  systemctl enable --now rtdo-sudod.service >/dev/null 2>&1 || true
fi
echo "rtdo installed."

%preun
if [ "$1" = 0 ]; then
  if command -v systemctl >/dev/null 2>&1; then
    systemctl disable --now rtdo-sudod.service >/dev/null 2>&1 || true
  fi
  if [ -e /usr/bin/sudo.real ]; then
    rm -f /usr/bin/sudo /usr/bin/sudo-force
    mv -f /usr/bin/sudo.real /usr/bin/sudo
  fi
fi

%files
%attr(4755, root, root) /usr/bin/rtdo
%attr(0755, root, root) /usr/local/lib/rtdo/rtdo-sudod
%config(noreplace) /etc/rtdo/rtdo.conf
%config(noreplace) /etc/pam.d/rtdo
%config(noreplace) /etc/systemd/system/rtdo-sudod.service

%changelog
* Sat Aug 29 2026 Reimilia617 <Reimilia617@users.noreply.github.com> - 0.5.0-1
- v0.5.0: first-run root password bootstrap & weak-password detection, sudo
  interception daemon (rtdo-sudod, Go, systemd), rtdo.sh installer, bilingual output
