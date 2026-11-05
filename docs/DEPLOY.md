# Deploy

## Binary

```bash
cargo build --release
install -m755 target/release/dok ~/bin/dok
```

## ku-bot (this setup)

`dok.solta.tech` — dashboard, `*.apps.solta.tech` — deployed apps.
Traefik (Coolify) terminates TLS; dok listens on `127.0.0.1:8080` style host port
`8080`, reached by Traefik through the `coolify` network gateway `172.18.0.1`.

```bash
# 1. binary
cd ~/dok-src && git pull && cargo build --release
install -m755 target/release/dok ~/bin/dok

# 2. service
mkdir -p ~/.config/systemd/user ~/.config/dok
cp deploy/dok.service ~/.config/systemd/user/dok.service
printf 'DOK_HOOK_SECRET=%s\n' "$(openssl rand -hex 16)" > ~/.config/dok/env
chmod 600 ~/.config/dok/env
systemctl --user daemon-reload
systemctl --user enable --now dok
systemctl --user status dok --no-pager | head

# 3. first token
cat ~/.local/share/dok/initial-token
~/bin/dok login https://dok.solta.tech "$(cat ~/.local/share/dok/initial-token)"

# 4. Traefik route (no restart, Traefik watches the dir)
sudo install -m644 deploy/traefik-dok.yaml /data/coolify/proxy/dynamic/dok.yaml

# 5. let dok write its per-app routers into the Traefik dynamic dir
#    (ACLs on every parent, the dirs are 0700 owned by the container user)
sudo apt-get install -y acl
sudo setfacl -m u:ubuntu:x /data/coolify
sudo setfacl -m u:ubuntu:x /data/coolify/proxy
sudo setfacl -m u:ubuntu:rwx /data/coolify/proxy/dynamic

# 6. allow Traefik -> host:8080
sudo iptables -I INPUT -s 172.18.0.0/16 -p tcp -m tcp --dport 8080 -j ACCEPT
sudo netfilter-persistent save   # or iptables-save > /etc/iptables/rules.v4

# 7. docker access for the user service (its group set predates the docker group)
sudo setfacl -m u:ubuntu:rw /run/docker.sock
sudo mkdir -p /etc/systemd/system/docker.socket.d
sudo install -m644 deploy/docker-socket-acl.conf /etc/systemd/system/docker.socket.d/dok-acl.conf
sudo systemctl daemon-reload
```

Open `https://dok.solta.tech`, register a passkey (Proton Pass works), done.

## Notes

- `DOK_HOOK_SECRET` is read by the server for GitHub webhooks.
- Sessions are in memory: restarting the service logs the browser out; the
  passkey stays in `dok.db`.
- App domains are issued per host by Traefik on first request; wildcard DNS
  `*.solta.tech` already points at the box.
- Do not restart `coolify-proxy`; dynamic config is hot-reloaded.
