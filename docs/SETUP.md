# Setting up: linking two laptops

Each DJ runs Face2Face DJ on their own laptop. Once linked, each window shows
the other DJ's decks. Setting up takes three steps: put both laptops on one
network, exchange an address and a code, and choose where your decks come from.
You do the first two once; the app remembers them.

Status of this guide: linking was tested on one Mac, with the app connecting
over that Mac's network address to a second Sender. It has not yet been run
between two physical laptops, on Windows, or from an installed (packaged) app.

## 1. Put both laptops on the same network

Any one of these. The app does not need the internet, only a path between the
two laptops.

**A cable between the two laptops** (the most reliable in a booth)

1. Connect an Ethernet cable from one laptop to the other. Laptops without an
   Ethernet port need a USB-C or Thunderbolt Ethernet adapter. Any Ethernet
   cable works.
2. Wait up to a minute. Each laptop gives itself an address starting with
   `169.254.`; there is nothing to configure. (macOS shows it in System Settings >
   Network as "Self-assigned IP". That is expected here.)
3. Wi-Fi can stay on for internet. The app then shows two addresses for the
   laptop; give your partner the `169.254.` one.

**The same Wi-Fi**

Join both laptops to the same Wi-Fi network. Home and studio routers work.
Venue and guest networks often stop devices from reaching each other ("client
isolation"); if linking never completes on one, use a cable or a hotspot.

**A phone hotspot**

Turn on the hotspot on one phone and join both laptops to it.
iPhone: Settings > Personal Hotspot > Allow Others to Join. Android: Settings >
Network & internet > Hotspot & tethering > Wi-Fi hotspot. The decks' data goes
from laptop to laptop through the phone and uses no mobile data.

**The booth's network switch**

If the booth has a switch for the CDJs (Pro DJ Link), plugging both laptops
into it also works, as long as both get an address on it.

## 2. Link

On **both** laptops:

1. Start Face2Face DJ and choose **Link up**.
2. The window shows this laptop's address and its pairing code, for example
   `This laptop: 192.168.1.70 · code 483 920`. Tell your partner both.
3. Type your partner's address and code into the two boxes and press **Link**.

When both have done it, each window shows the other's decks, and the status
at the top right says `Network` with the delay in milliseconds.

The first time, the operating system may ask for permission. Allow it,
otherwise your partner cannot connect:

- macOS: "Do you want the application Face2Face DJ to accept incoming network
  connections?" (only when the macOS firewall is on), and "Face2Face DJ would
  like to find and connect to devices on your local network". Both can be
  changed later in System Settings > Network > Firewall, and Privacy & Security >
  Local Network.
- Windows: the Windows Defender Firewall dialog. Allow on private networks.

What is remembered: your partner's address and code, and your own code (it
never changes, so your partner types it once). What can change: **addresses**.
On Wi-Fi and hotspots a laptop may get a different address next time. If the
window stays on "Waiting for your partner", press **Partner** in the top bar and
enter the address your partner's window shows now.

### One-way

**Share only** on one laptop and **View only** on the other. The sharing laptop
shows its address and code in the top bar. On the viewing laptop press
**Partner**, then enter them.

### Trying it on one computer

Open the app twice, choose **Link up** in both, and in each press **My partner
is on this computer**. Each window shows the other's mock decks.

## 3. Choose where your decks come from

In the top bar, the menu at the left chooses the source: mock decks, Traktor,
rekordbox, Pro DJ Link or Mixxx. Each needs something on the DJ software's side;
see [SOURCES.md](SOURCES.md).

## If it does not link

| The window says | Check |
|---|---|
| "This laptop is not on a network" | Step 1: join the Wi-Fi or hotspot, or plug in the cable and wait a minute. |
| "Waiting for your partner", for more than a few seconds | Both laptops on the same network? Address typed exactly as your partner's window shows it now? Permission dialogs allowed on your partner's laptop? Has your partner chosen Link up? On venue Wi-Fi, try a cable or hotspot. |
| "Wrong pairing code" | Press **Correct the code** and enter the code from your partner's window. |
| Decks appear, then "Disconnected – showing last known" | The network dropped. The app reconnects by itself when it is back. |
| It links, but the other window shows nothing of yours | Your own source is not delivering: look at the status at the top left of your window, and at [SOURCES.md](SOURCES.md). |

## What the pairing code does and does not do

Without your code nobody on the network can read your decks from the app, and
guessing is limited to one try a second. The code and the deck data travel
unencrypted, like most traffic on a local network: someone able to record the
network's traffic could read both. What is sent is what is on your decks
(titles, tempo, position); no audio, no library.
