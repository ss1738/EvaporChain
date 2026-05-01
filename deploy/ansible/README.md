# EvaporChain Ansible deploy

Public-testnet operator surface for a 3-validator Hetzner Cloud cluster.
Designed to be the same playbooks that ship to mainnet — only inventory
and the coordinator-signed genesis change between environments.

## Prerequisites

- Ansible 2.15+ on the operator workstation (`ansible --version`).
- `community.general` and `ansible.posix` collections:
  `ansible-galaxy collection install community.general ansible.posix`.
- SSH access (key-based) to each Hetzner host.
- The coordinator-signed `genesis-config.json` produced by
  `evaporchain onboarding build-genesis`, placed at
  `deploy/ansible/files/genesis-config.json`.
- Per-validator BLS key bundles in
  `deploy/ansible/files/validator-keys/validator-<id>-bls-key.bin`,
  encrypted in EVK1 format via `evaporchain encrypt-bls-key`.
- Hetzner Cloud servers provisioned (CX32 or larger, Ubuntu 24.04 LTS),
  with the operator's SSH key pre-installed.

## One-time setup

```sh
cd deploy/ansible
ansible-vault create group_vars/vault.yml
# Inside the editor, write:
#   evaporchain_validator_key_pass: "<strong-passphrase>"
```

Edit `inventory/testnet.yml` and replace each `REPLACE_ME_VALIDATOR_<n>_IP`
with the real Hetzner IP. Edit `group_vars/all.yml` to fill in the
`snapshot_backup_*` placeholders if you intend to use the snapshot
playbook.

## Bootstrap

```sh
ansible-playbook playbooks/bootstrap.yml --ask-vault-pass
```

Idempotent. Installs base packages, creates the `evaporchain` user,
configures ufw + sysctl + swap, generates per-host TLS material, and
drops the systemd unit (service stays stopped until the binary is
deployed).

## Deploy

```sh
ansible-playbook playbooks/deploy.yml -e release_tag=v0.2.0 --ask-vault-pass
```

Pulls the GitHub release tarball, verifies its SHA256, installs the
binary, uploads the genesis-config and encrypted BLS key, renders
`node.toml`, and waits for `/api/status` to return 200 within 60s.

## Rolling upgrade

```sh
ansible-playbook playbooks/upgrade.yml -e release_tag=v0.2.1 --ask-vault-pass
```

`serial: 1` — one validator at a time. Captures pre-upgrade height,
drains peers, swaps the binary, restarts, then waits up to 5 min for
the height to advance past the pre-upgrade mark before moving on. Any
host that fails to resync aborts the run.

## Snapshot

```sh
ansible-playbook playbooks/snapshot.yml --ask-vault-pass
```

`serial: 1` — never lose quorum. Stops one node, tars+zstd the data
dir, restarts, then rsyncs the snapshot to the configured backup host.
See the playbook header for the CLI-snapshot TODO once the
`evaporchain snapshot create` subcommand lands.

## Stop / start

```sh
ansible-playbook playbooks/stop.yml --ask-vault-pass    # graceful, serial: 1
ansible-playbook playbooks/start.yml --ask-vault-pass   # parallel, with /api/status smoke
```

## Troubleshooting

- Tail logs:
  `ansible -m shell -a 'journalctl -u evaporchain-node -n 200 --no-pager' validators`
- Restart a single host:
  `ansible-playbook playbooks/start.yml -l validator-1 --ask-vault-pass`
- Check height + peer count cluster-wide:
  `ansible -m uri -a 'url=http://127.0.0.1:8080/api/status return_content=yes' validators`
- Roll back: `ansible-playbook playbooks/upgrade.yml -e release_tag=<previous-tag> --ask-vault-pass`
- Force-stop a stuck node (manual):
  `ansible -m systemd -a 'name=evaporchain-node state=stopped' -b validator-2`

## Layout

```
deploy/ansible/
  ansible.cfg
  inventory/{testnet.yml,README.md}
  group_vars/{all,validators,vault}.yml
  playbooks/{bootstrap,deploy,start,stop,upgrade,snapshot}.yml
  roles/{common,evaporchain-node,prometheus-exporter,tls}/
```
