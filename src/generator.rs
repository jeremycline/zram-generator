/* SPDX-License-Identifier: MIT */

use crate::config::Device;
use anyhow::{anyhow, Context, Result};
use std::borrow::Cow;
use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn make_parent(of: &Path) -> Result<()> {
    let parent = of
        .parent()
        .ok_or_else(|| anyhow!("Couldn't get parent of {}", of.display()))?;
    fs::create_dir_all(&parent)?;
    Ok(())
}

fn make_symlink(dst: &str, src: &Path) -> Result<()> {
    make_parent(src)?;
    symlink(dst, src).with_context(|| {
        format!(
            "Failed to create symlink at {} (pointing to {})",
            dst,
            src.display()
        )
    })?;
    Ok(())
}

fn virtualization_container() -> Result<bool> {
    match Command::new("systemd-detect-virt")
        .arg("--container")
        .stdout(Stdio::null())
        .status()
    {
        Ok(status) => Ok(status.success()),
        Err(e) => Err(anyhow!("systemd-detect-virt call failed: {}", e)),
    }
}

pub fn run_generator(
    root: Cow<'static, str>,
    devices: Vec<Device>,
    output_directory: PathBuf,
) -> Result<()> {
    if devices.is_empty() {
        println!("No devices configured, exiting.");
        return Ok(());
    }

    if virtualization_container()? {
        println!("Running in a container, exiting.");
        return Ok(());
    }

    let mut devices_made = false;
    for dev in &devices {
        devices_made |= handle_device(&output_directory, dev)?;
    }
    if devices_made {
        /* We created some devices, let's make sure the module is loaded and creation service is present */
        make_service_template(&output_directory)?;

        let modules_load_path = Path::new(&root[..]).join("run/modules-load.d/zram.conf");
        make_parent(&modules_load_path)?;
        fs::write(&modules_load_path, "zram\n").with_context(|| {
            format!(
                "Failed to write configuration for loading a module at {}",
                modules_load_path.display()
            )
        })?;
    }

    Ok(())
}

fn make_service_template(output_directory: &Path) -> Result<()> {
    let service_path = output_directory.join("swap-create@.service");

    let contents = format!(
        "\
# Automatically generated by zram-generator

[Unit]
Description=Create swap on /dev/%i
Wants=systemd-modules-load.service
After=systemd-modules-load.service
After=dev-%i.device
DefaultDependencies=false

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStartPre=-modprobe zram
ExecStart={generator} --setup-device '%i'
",
        generator = env::current_exe()
            .context("Couldn't get path to generator executable")?
            .display(),
    );
    fs::write(&service_path, contents).with_context(|| {
        format!(
            "Failed to write a device service into {}",
            service_path.display()
        )
    })?;

    Ok(())
}

fn handle_device(output_directory: &Path, device: &Device) -> Result<bool> {
    let swap_name = format!("dev-{}.swap", device.name);
    println!(
        "Creating {} for /dev/{} ({}MB)",
        swap_name,
        device.name,
        device.disksize / 1024 / 1024
    );

    let swap_path = output_directory.join(&swap_name);

    let contents = format!(
        "\
# Automatically generated by zram-generator

[Unit]
Description=Compressed swap on /dev/{zram_device}
Requires=swap-create@{zram_device}.service
After=swap-create@{zram_device}.service

[Swap]
What=/dev/{zram_device}
Priority=100
",
        zram_device = device.name
    );
    fs::write(&swap_path, contents).with_context(|| {
        format!(
            "Failed to write a swap service into {}",
            swap_path.display()
        )
    })?;

    let symlink_path = output_directory.join("swap.target.wants").join(&swap_name);
    let target_path = format!("../{}", swap_name);
    make_symlink(&target_path, &symlink_path)?;
    Ok(true)
}
