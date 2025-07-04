# dynv6-sync

Daemon-less Dynv6 sync program

## Abstruct

`dynv6-sync` is a lightweight, daemon-less synchronization tool for the Dynv6 dynamic DNS service. It supports dynamic configuration changes via variable updates, making it suitable for environments where assigned addresses, available ports, and other network parameters frequently change.

## Description

Although Dynv6 provides REST API for dynamic DNS updates, its records are volatile - the service frequently and inexplicably drop records. (This issue seems to stem from inconsistencies across Dynv6's distributed name servers) Importantly, it affects not only A and AAAA records, but also CNAME, SRV, TXT, and other types.

Most existing Dynv6 clients only handle A/AAAA records, leaving full DNS configurations vulnerable to this instability. In addition, there are use cases where non-address records must be updated dynamically—such as SPF settings or DNS-01 challenges for Let’s Encrypt.

`dynv6-sync` addresses these problems in the following ways:

- Full configuration synchronization
  On every run, `dynv6-sync` try to ensure the perfect consistency of the DNS configuration. It compares the ideal state (as defined in the config) against Dynv6’s actual state and applies only the necessary changes, minimizing API calls.
- Runtime-evaluated variables
  Config files can reference external values (e.g. IP addresses, ports, ...) through variables. These variables are stored separately and resolved at runtime, allowing clean separation between static structure and dynamic data. Variable values can be updated independently, without triggering a sync.

Since [this program is just a stopgap](#dynv6-sync-is-a-stopgap), it intentionally lacks daemon features—especially scheduling. The intended usage is to run it from external schedulers (e.g. systemd.timer, cron, ...) or event hooks (e.g. `NetworkManager` dispatcher script, `/etc/network/if-up.d/`, ...).

Supported platforms: Linux and macOS

## Installation

Currently, only a Debian repository for Debian is available.

### Debian

Supported Codename: `bullseye`, `bookworm`

Supported Architecture: `arm64`, `amd64`

```bash
curl -fsSL https://jyjyjcr.github.io/dynv6-sync/publish/gpg.key.asc | sudo gpg --dearmor -o /etc/apt/keyrings/dynv6-sync.gpg
sudo echo "deb [signed-by=/etc/apt/keyrings/dynv6-sync.gpg] https://jyjyjcr.github.io/dynv6-sync/publish/deb $(cat /etc/os-release|grep VERSION_CODENAME|sed -e 's/^.*=//g') main" > "/etc/apt/sources.list.d/dynv6-sync.list"
sudo apt update
sudo apt install dynv6-sync
```

## Usage

```console
> dynv6-sync --help
Usage: dynv6-sync [OPTIONS] <CONF>

Arguments:
  <CONF>  Path to the configuration file

Options:
  -L, --log-out <LOG_OUT>               [default: stdout] [possible values: stdout, journald]
  -l, --log-level <LOG_LEVEL>           [default: info]
  -u, --update <KEYVALUES> <KEYVALUES>  Update variable
      --nosync                          Do not sync the records, just update variables
  -h, --help                            Print help
```

Configuration Example:

`sync.json`:

```json
{
    "vars_path": "sync.vars.json", # variable file
    "token_path": "sync.secret.json",# secret token file
    "lock_path": "sync.lock", # lock file
    "domain": "example-example.dynv6.net", # domain
    "retry": 3, # retry limit if synchronization is incomplete
    "records": [
        {
            "name": "",
            "A": {
                "addr": "${gate.v4}" # you can reference variables as `${varname}`
            }
        },
        {
            "name": "",
            "AAAA": {
                "addr": "${gate.v6}"
            }
        },
        {
            "name": "game",
            "A": {
                "addr": "${game.v4}"
            }
        },
        {
            "name": "game",
            "AAAA": {
                "addr": "${game.v6}"
            }
        },
        {
            "name": "www",
            "CNAME": {
                "domain": ""
            }
        },
        {
            "name": "vpn",
            "CNAME": {
                "domain": ""
            }
        },
        {
            "name": "_minecraft._tcp.${game.mc1.name}.game", # you can use variables in record name
            "SRV": {
                "domain": "game",
                "port": "${game.mc1.port}",
                "priority": "1",
                "weight": "1"
            }
        },
        {
            "name": "_minecraft._tcp.${game.mc2.name}.game",
            "SRV": {
                "domain": "game",
                "port": "${game.mc2.port}",
                "priority": "1",
                "weight": "1"
            }
        },
        {
            "name": "_minecraft._tcp.mc3.game",
            "SRV": {
                "domain": "game",
                "port": "${game.mc3.port}",
                "priority": "1",
                "weight": "1"
            }
        }
    ]
}
```

`sync.vars.json`:

```json
{
    "gate.v4": "192.168.0.1",
    "gate.v6": "fe80::1",
    "game.v4": "192.168.0.2",
    "game.v6": "fe80::2",
    "game.mc1.port":"25565",
    "game.mc2.port":"25566",
    "game.mc3.port":"25567",
    "game.mc1.name":"lobby",
    "game.mc2.name":"pvp",
}
```

`sync.secret.json`:

```json
"PutYourTokenHereFromDynv6Site"
```

## Use Case

You can use `--nosync` to update dynamic variables without triggering DNS updates, which is suitable for network event hooks.

Example NetworkManager dispatcher script:

```sh
#!/bin/sh -e
if [ "$NM_DISPATCHER_ACTION" = "up" ];then
    if [ "$CONNECTION_UUID" = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" ];then
        ipv4="$(/usr/bin/upnpc -s 2>/dev/null |/usr/bin/grep "ExternalIPAddress = "|/usr/bin/sed "s/ExternalIPAddress = //g")"
        ipv6="$(/usr/sbin/ip a show dev "$DEVICE_IP_IFACE" scope global 2>/dev/null|/usr/bin/grep inet6|/usr/bin/sed -e "s/^.*inet6 //g" -e "s/\/.*//g")"
        /usr/bin/dynv6-sync /opt/ddns/sync.json -L journald --nosync -u "ipv4" "$ipv4" -u "ipv6" "$ipv6"
    fi
fi
```

Note: Logs will appear under `NetworkManager-dispatcher.service`.

## dynv6-sync is a stopgap

As its minimalist and somewhat quirky design suggests, dynv6-sync is only intended as a stopgap until a more sophisticated daemon is available. It only implements the essential features needed for stable operation, and no additional convenience features are planned.

For example, a fully featured daemon might support behavior such as: when a variable changes, wait for 10 seconds; if a sync is triggered during that time, cancel it; if another variable update occurs, postpone the sync again; otherwise, proceed. This would allow for near-instant synchronization on IP changes. In contrast, dynv6-sync relies on a simple periodic timer as a workaround.

Development of a more advanced daemon is underway in the [dyind project](https://github.com/JyJyJcr/dyind). Contributions are welcome.
