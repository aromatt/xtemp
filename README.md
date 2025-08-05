# xtemp
Like `xargs`, but for utilities that can process batches of items only if they are
provided as files, e.g. `md5sum`.

`xtemp` copies items from stdin to a pool of temporary files in batches, invoking the
provided command for each batch.

## Purpose
Many Unix utilities operate naturally over streams of independent records (e.g.
`sed`, `grep`). But some, like `wc` and `md5sum`, do not, even though they
provide functionality that can still be useful in these contexts.

`xtemp` acts as an adapter for these tools, allowing them to be used more efficiently
in pipelines.

## Usage
```
Usage: xtemp [OPTIONS] [COMMAND]...

Arguments:
  [COMMAND]...  Command to execute with tempfile arguments

Options:
  -n, --batch-size <BATCH_SIZE>  Number of lines per batch. xtemp will write batch_size
                                 lines to batch_size tempfiles, and pass those tempfiles
                                 as arguments to the command
  -J, --replstr <REPLSTR>        Replacement string for tempfile arguments. If not
                                 specified, tempfiles are appended as trailing arguments
  -h, --help                     Print help
  -V, --version                  Print version
```

## Example: Calculating hashes line-by-line
If you pipe multiple lines to `md5sum`, it treats them all as a single message and
outputs just one hash:
```bash
$ echo -e "foo\nbar" | md5sum
f47c75614087a8dd938ba4acff252494  -
```

If you want to generate an MD5 for each line, you have to resort to spawning a
new process for each line of input:
```bash
$ echo -e "foo\nbar" | while read line; do echo "$line" | md5sum; done
d3b07384d113edec49eaa6238ad5ff00  -
c157a79031e1c40f85931829bc5fc552  -
```

But `md5sum` _is_ capable of calculating multiple hashes in one execution; you just
have to provide the inputs in separate files:
```bash
$ echo foo > foo.txt; echo bar > bar.txt
$ md5sum foo.txt bar.txt
d3b07384d113edec49eaa6238ad5ff00  foo.txt
c157a79031e1c40f85931829bc5fc552  bar.txt
```

`xtemp` does this for you, using a pool of temporary files behind the scenes:
```bash
$ echo -e "foo\nbar" | xtemp md5sum
d3b07384d113edec49eaa6238ad5ff00  /tmp/.tmpFUVzUN
c157a79031e1c40f85931829bc5fc552  /tmp/.tmpFUVzUN
```

### Performance
In practice, using `xtemp` instead of a process per line is much faster:
```bash
# Generate input data
$ < /dev/urandom tr -dc '0-9a-z' | fold -w 10 | head -10000 > sample.10k.txt

# Using xtemp
$ time xtemp md5sum < sample.10k.txt >/dev/null

real    0m0.928s
user    0m0.165s
sys     0m0.676s

# Spawning a process per line
$ time sh -c 'while read line; do echo "$line" | md5sum; done' < sample.10k.txt >/dev/null

real    0m12.828s
user    0m9.306s
sys     0m3.309s
```
