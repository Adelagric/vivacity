# vivacity in a Docker image build

A `docker build` that installs PHP dependencies runs `composer install` into
an empty `vendor/` every time the dependency layer is rebuilt. With
Composer's files cache on a BuildKit cache mount, that is the case
`bench/M7-ci-cache.md` measures: the zips are warm, nothing else is. There
vivacity builds its store from Composer's own zips on the fly, so nothing
about the cache changes.

| install into an empty `vendor/`, Sylius (276 packages), ubuntu-latest | median |
|---|---|
| `composer install`, zips warm | 5 373 ms |
| `vivacity install`, zips warm, store cold | 2 145 ms |

Same machine class as a CI worker (x86_64, 4 vCPU, ext4), hyperfine, 5
runs, 2026-09-16; method and raw numbers in
[bench/M7-ci-cache.md](../bench/M7-ci-cache.md).
A real image build adds its own overhead around it: measure yours.

## Dockerfile

```dockerfile
# syntax=docker/dockerfile:1
FROM php:8.4-cli
# Any glibc 2.17+ image; on an Alpine image (php:*-alpine) install.sh picks
# the static musl build, after `apk add curl`.
RUN apt-get update && apt-get install -y --no-install-recommends curl unzip \
 && rm -rf /var/lib/apt/lists/*
# Composer stays in the image: vivacity hands it what it does not reproduce,
# and runs composer.json's scripts through it with --run-scripts.
COPY --from=composer:2.10.3 /usr/bin/composer /usr/local/bin/composer
RUN curl -fsSL https://raw.githubusercontent.com/Adelagric/vivacity/main/install.sh \
  | VIVACITY_VERSION=v0.25.0 VIVACITY_INSTALL_DIR=/usr/local/bin sh

WORKDIR /app
ENV COMPOSER_CACHE_DIR=/cache/composer

# Dependencies first, so this layer is rebuilt only when the lock changes.
COPY composer.json composer.lock ./
RUN --mount=type=cache,target=/cache/composer \
    VIVACITY_CACHE_DIR=/tmp/vivacity vivacity install \
      --no-dev --prefer-dist --no-interaction --no-autoloader \
 && rm -rf /tmp/vivacity

# Then the application, and the autoloader with the scripts that need it.
COPY . .
RUN vivacity dump-autoload --optimize --no-dev --run-scripts
```

Why these choices:

- **The store lives and dies inside the `RUN`.** vivacity lays packages out
  by hardlinking (or reflinking) them from its store; in the same layer that
  costs nothing, and removing the store at the end of the `RUN` leaves the
  files in `vendor/` and nothing extra in the image. Putting the store on a
  cache mount instead would place it on another filesystem: vivacity then
  copies the files (same result, slower), and the gain shrinks to skipping
  the extraction.
- **Only Composer's files cache is mounted**, at `COMPOSER_CACHE_DIR`. It is
  the cache both tools read, and the one `bench/M7-ci-cache.md` found worth
  keeping between builds.
- **Scripts.** vivacity never runs `composer.json` scripts on its own;
  `--run-scripts` runs `pre-autoload-dump` / `post-autoload-dump` through
  `composer run-script` at Composer's points, which is what Laravel's
  `package:discover` and Symfony's `auto-scripts` hang on. `--no-autoloader`
  in the dependency layer keeps the classmap for the step that has the
  application code.
- **`-v` / `--verbose`** stays native with the plugins whose verbose lines
  vivacity knows (a standard Laravel project's among them); a project with
  symfony/flex, dealerdirect's PHP_CodeSniffer installer or another plugin
  outside that list goes to Composer under `-v`, and the build gains nothing.
  Leaving `-v` out avoids the question.
- **Pin the version** (`VIVACITY_VERSION`), as the image pins Composer: the
  install script checks the sha256 of the release asset it downloads.

## Checked

The Dockerfile above, built on the Laravel fixture of this repository
(`php:8.4-cli`, linux/arm64 under colima, BuildKit v0.27.1, 2026-10-22):
76 packages laid out natively, `package:discover` run by
`dump-autoload --run-scripts`, `php artisan --version` answering in the
image, no store left behind. Rebuilding the dependency layer with the zips
already on the cache mount: 0.46 s for `vivacity install`, against 1.5–1.7 s
when they come from the network.

`docker build --no-cache` recreated the cache mounts too on that builder:
every run then downloads again. To time a rebuild, change the lock (or a
build argument before the `COPY`) instead.

## What to expect when vivacity hands over

A lock vivacity does not lay out natively (an unported plugin, a `git`-only
package, the Drupal scaffold…) goes to the `composer` in the image, with the
reason on stderr, before anything is written: the build still succeeds, at
Composer's speed. `vivacity install --check-scope` (no network, nothing
written) tells you in advance; exit 0 means native.
