# Inventory

Three-validator public-testnet topology. Edit `testnet.yml` after Hetzner
provisioning is complete.

## Swapping the placeholders

Open `inventory/testnet.yml` and replace each `REPLACE_ME_VALIDATOR_<n>_IP`
with the public IPv4 address Hetzner Cloud assigned to that server. Both
`ansible_host` (used by Ansible to reach the box) and `p2p_external_host`
(advertised to peers in node config) must be set; for a flat single-NIC
Hetzner Cloud node these are usually identical. If the host is behind a
load balancer or has separate management vs. public NICs, set the
management IP for `ansible_host` and the publicly-reachable IP for
`p2p_external_host`.

The `validator_id` field is 0-indexed and MUST line up with the order
the coordinator used when running `evaporchain onboarding build-genesis`.
Renumbering after the genesis is signed will cause every validator to
fail the BLS-key-vs-genesis check at startup.

## Adding a fourth validator

1. Provision the new Hetzner Cloud server (CX32 or larger, Ubuntu 24.04).
2. Append a host block under `validators.hosts`:
   ```yaml
   validator-3:
     ansible_host: <new-ip>
     validator_id: 3
     validator_moniker: delta
     p2p_external_host: <new-ip>
   ```
3. Generate that validator's BLS keys with `evaporchain keygen --output validator-3.json`
   on the secure operator workstation.
4. Re-issue the coordinator-signed genesis with the expanded validator
   manifest (`evaporchain onboarding build-genesis ...`). Every existing
   validator must restart against the new genesis-config; this is a
   coordinated network upgrade, not a hot-add.
5. Update `validator_count: 4` in `group_vars/validators.yml`.
6. `ansible-playbook playbooks/bootstrap.yml -l validator-3 --ask-vault-pass`,
   then `ansible-playbook playbooks/deploy.yml -e release_tag=<tag> --ask-vault-pass`.

## SSH access

The bootstrap playbook creates the `evaporchain` system user with a
locked password and authorized_keys populated from the operator's local
public key. The first run must be executed as `root` (or another user
with sudo) by passing `-u root --become` if the cloud image does not
already grant the `evaporchain` user sudo. After bootstrap, all
subsequent plays run as the `evaporchain` user with passwordless sudo
restricted to systemctl and apt operations.
