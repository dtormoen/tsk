import click

@click.command()
@click.option("--name", default="Python integration test", help="Name to greet")
def hello(name):
    click.echo(f"Hello from {name}!")
    click.echo("SUCCESS: Python stack is working")

if __name__ == "__main__":
    hello()
