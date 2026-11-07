#!/usr/bin/env ruby
# Runs the parity job's commands, read from ci.yml so that none is left out, and
# carries on past a failure. Each command gets a LEDGER line with its exit code; a
# command whose prerequisite failed is not run and says which one. Exits 0 only when
# every planned command ran and passed. No LEDGER-END line means the run was cut short.
#
# usage: parity-ledger.rb <ci.yml> [--plan]
require "yaml"

Encoding.default_external = Encoding::UTF_8

ci, mode = ARGV
abort "usage: #{$PROGRAM_NAME} <ci.yml> [--plan]" unless ci
$stdout.sync = true

BUILD = "cargo build --release".freeze
FIXTURES = "fixtures/make.sh".freeze
TEST = "cargo test".freeze
KNOWN = %r{\A(cargo |fixtures/make\.sh\z|tools/|harness/)}

steps = YAML.load_file(ci).fetch("jobs").fetch("parity").fetch("steps")
plan = []
steps.each do |step|
  next unless step["run"]
  # Read at both ends of the workflow itself: they need the step's own token.
  next if step["name"].start_with?("GitHub API budget")

  lines = step["run"].lines.map(&:strip).reject(&:empty?)
  lines.each_with_index do |line, i|
    abort "not a command this ledger knows: #{line}" unless line.match?(KNOWN)

    needs = []
    case line
    when TEST then needs << FIXTURES
    when %r{\Atools/check-pcre2-symbols} then needs << BUILD
    when %r{\Aharness/}
      needs << BUILD
      script = line.split.find { |w| w.start_with?("harness/") && !w.end_with?("net-retry.sh") } || line.split.first
      # Too wide, knowingly: mentioning fixtures is not needing them, and a script's
      # lib/ is not read. If the preparation fails, some harnesses that stand alone are
      # NOT-RUN too, and the ledger is then incomplete. It does not matter while make.sh
      # passes.
      needs << FIXTURES if File.read(script).match?(/fixture/i)
      # As in the job, where the step's shell stops at its first failure. By place,
      # not by text: the cold and warm runs of a step are the same command.
      needs << plan.size - 1 if i.positive?
    end
    plan << { line: line, run: line == TEST ? "#{TEST} --no-fail-fast" : line, needs: needs }
  end
end
place = {}
[BUILD, FIXTURES, TEST].each do |needed|
  found = plan.each_index.select { |i| plan[i][:line] == needed }
  abort "expected one `#{needed}` in #{ci}, found #{found.size}" unless found.size == 1
  place[needed] = found.first
end
plan.each { |item| item[:needs] = item[:needs].map { |n| place.fetch(n, n) } }
label = ->(i) { "#{plan[i][:run]} (##{i + 1})" }

puts "LEDGER-PLAN #{plan.size} commands from #{ci}"
plan.each_with_index do |item, i|
  needs = item[:needs].empty? ? "" : "  (needs: #{item[:needs].map(&label).join(', ')})"
  puts "  ##{i + 1} #{item[:run]}#{needs}"
end
exit 0 if mode == "--plan"

outcome = []
ledger = []
plan.each_with_index do |item, i|
  missing = item[:needs].reject { |n| outcome[n] == :ok }
  if missing.empty?
    puts "::group::#{item[:run]}"
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    pid = spawn("bash", "-e", "-c", item[:run])
    Process.wait(pid)
    status = $?
    seconds = (Process.clock_gettime(Process::CLOCK_MONOTONIC) - started).round
    puts "::endgroup::"
    code = status.exited? ? status.exitstatus.to_s : "signal-#{status.termsig}"
    outcome[i] = status.success? ? :ok : :failed
    entry = "LEDGER #{status.success? ? 'ok' : 'FAILED'} rc=#{code} #{seconds}s | ##{i + 1} #{item[:run]}"
  else
    outcome[i] = :not_run
    entry = "LEDGER NOT-RUN needs=[#{missing.map(&label).join(', ')}] | ##{i + 1} #{item[:run]}"
  end
  ledger << entry
  puts entry
end

count = ->(what) { outcome.count(what) }
puts "LEDGER-SUMMARY"
ledger.each { |entry| puts entry }
puts "LEDGER-END planned=#{plan.size} ok=#{count[:ok]} failed=#{count[:failed]} not-run=#{count[:not_run]}"
exit(count[:ok] == plan.size ? 0 : 1)
