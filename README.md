# xtemp
`xtemp` is a command-line utility that temporarily materializes stdin lines as files
so that tools like `md5sum` can be used more efficiently in shell pipelines.

## Purpose
Many Unix utilities operate naturally over streams of items (e.g. `sed`, `grep`). But
some, like `md5sum`, do not, even though they could otherwise be useful in these
contexts.

`xtemp` acts as an adapter allowing file-batch processors to be used in line-based
stream-processing pipelines.

Under the hood, `xtemp` opens a pool of temporary files, then repeatedly executes the
provided command in batches, passing the set of temporary files as arguments each
time.

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
      --keep-newlines            Keep newlines when writing lines to tempfiles (default:
                                 strip newlines)
  -h, --help                     Print help
  -V, --version                  Print version
```

## Illustrative example: calculating hashes line-by-line
If you pipe multiple lines to `md5sum`, it treats them all as a single message and
outputs just one hash:
```bash
$ echo -e "foo\nbar" | md5sum
f47c75614087a8dd938ba4acff252494  -
```

If you want to generate an MD5 for each line, you have to resort to spawning a
new process for each line of input:
```bash
$ echo -e "foo\nbar" | while read line; do printf "$line" | md5sum; done
acbd18db4cc2f85cedef654fccc4a4d8  -
37b51d194a7513e45b56f6524f2d51f2  -
```

But `md5sum` _is_ capable of calculating multiple hashes in one execution; you just
have to provide the inputs in separate files:
```bash
$ printf foo > foo.txt; printf bar > bar.txt
$ md5sum foo.txt bar.txt
acbd18db4cc2f85cedef654fccc4a4d8  foo.txt
37b51d194a7513e45b56f6524f2d51f2  bar.txt
```

`xtemp` does this for you, using a pool of temporary files behind the scenes:
```bash
$ echo -e "foo\nbar" | xtemp md5sum
acbd18db4cc2f85cedef654fccc4a4d8  /tmp/.tmpn6rIRI
37b51d194a7513e45b56f6524f2d51f2  /tmp/.tmpnJk5rQ
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
$ time sh -c 'while read line; do printf "$line" | md5sum; done' < sample.10k.txt >/dev/null

real    0m13.860s
user    0m9.764s
sys     0m3.828s
```
