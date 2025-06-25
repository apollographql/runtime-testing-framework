#! /usr/bin/bash

# Create our test environment after our test system has booted.
# Many of the commands used require sudo access, but the script itself should not be run as sudo

ID=$(id -u)
GID=$(id -g)
ID_NAME=$(id -un)

# Enable non-root docker
sudo usermod -aG docker "${ID_NAME}"

# Copy important resource from the 'router' user to our directory
sudo cp -r ~router/.cargo .; sudo chown -R "${ID}:${GID}" .cargo
sudo cp -r ~router/.rustup .; sudo chown -R "${ID}:${GID}" .rustup
sudo cp -r ~router/go .; sudo chown -R "${ID}:${GID}" go
sudo cp -r ~router/scale .; sudo chown -R "${ID}:${GID}" scale

# Configure our .bashrc contents
echo ". \${HOME}/.cargo/env" >> .bashrc
# Set PATH. Make sure our bits are at the front of the PATH
echo "export PATH=\${HOME}/scale/bin:\${HOME}/scale/scripts:\${HOME}/go/bin:\${PATH}" >> .bashrc
# Set Cargo Target to a single location to speed up re-compile
echo "mkdir -p /tmp/rust/target" >> .bashrc
echo "export CARGO_TARGET_DIR=/tmp/rust/target" >> .bashrc
# We like to open files
echo "ulimit -n 100000" >> .bashrc
# We'd like to have our .bashrc functionality available in our login shells
echo "source ~/.bashrc" > .bash_profile

# Enable perf for unprivileged users
echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid > /dev/null

# Tell ssh about github.com to prevent complaints during clone
ssh-keyscan github.com >> .ssh/known_hosts

# Wait for the docker service to start
while ! systemctl is-active docker; do
  echo "waiting for docker..."
  sleep 1
done

# Pull useful images into base machine image
sudo -g docker docker pull -q jaegertracing/all-in-one
sudo -g docker docker pull -q otel/opentelemetry-collector-contrib:0.103.1
sudo -g docker docker pull -q redis
