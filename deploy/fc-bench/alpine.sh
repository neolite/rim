# Обычная установка Alpine 3.24 для того же замера: linux-virt + initramfs + OpenRC + openssh,
# статическая сеть как у rim, ключи хоста заранее. Запуск: в контейнере alpine:3.24 (linux/amd64).
set -eu
apk add -q e2fsprogs mkinitfs openssh-keygen
R=/alp
apk -q --root $R --initdb -X https://dl-cdn.alpinelinux.org/alpine/v3.24/main --allow-untrusted \
  add alpine-base openssh-server linux-virt >/dev/null
KVER=$(ls $R/lib/modules)
cat > $R/etc/network/interfaces <<'N'
auto lo
iface lo inet loopback
auto eth0
iface eth0 inet static
  address 172.16.0.2/30
  gateway 172.16.0.1
N
echo nameserver 1.1.1.1 > $R/etc/resolv.conf
echo alpine-bench > $R/etc/hostname
echo virtio_net >> $R/etc/modules
echo '/dev/vda / ext4 rw,relatime 0 1' > $R/etc/fstab
sed -i 's/^tty[1-6]/#&/' $R/etc/inittab
echo 'ttyS0::respawn:/sbin/getty -L 115200 ttyS0 vt100' >> $R/etc/inittab
mkdir -p $R/root/.ssh && cp /in/key.pub $R/root/.ssh/authorized_keys
chmod 700 $R/root/.ssh && chmod 600 $R/root/.ssh/authorized_keys
ssh-keygen -q -A -f $R
for s in devfs dmesg mdev hwdrivers; do ln -sf /etc/init.d/$s $R/etc/runlevels/sysinit/$s; done
for s in modules sysctl hostname bootmisc networking; do ln -sf /etc/init.d/$s $R/etc/runlevels/boot/$s; done
ln -sf /etc/init.d/sshd $R/etc/runlevels/default/sshd
for s in mount-ro killprocs savecache; do ln -sf /etc/init.d/$s $R/etc/runlevels/shutdown/$s; done
mkinitfs -b $R -F "base virtio ext4" -o /out/alpine.initramfs $KVER >/dev/null
cp $R/boot/vmlinuz-virt /out/alpine.vmlinuz
grep -E '^CONFIG_(VIRTIO_MMIO|VIRTIO_MMIO_CMDLINE_DEVICES|ACPI)=' $R/boot/config-* > /out/alpine.mmio-config || true
mke2fs -q -F -t ext4 -L root -d $R /out/alpine.img 512M
